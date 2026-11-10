//! Native runtime ownership and construction.

mod address_publisher;
mod builder;
pub(crate) mod executor;
mod outbox;
mod registry;

use std::str::FromStr;
use std::sync::Arc;
use std::sync::{Mutex as StdMutex, PoisonError};

use async_trait::async_trait;
use futures_util::StreamExt;
use iroh::endpoint::{
    BindError, ConnectError, ConnectingError, Connection, RecvStream, SendStream,
};
use iroh::{Endpoint, EndpointAddr, EndpointId};
use thiserror::Error;
use tokio::runtime::Handle;
use tokio::sync::{Mutex, broadcast, mpsc};

use self::address_publisher::PublishedAddresses;
use self::outbox::{Outbound, Outbox};
use self::registry::DeviceRegistry;
use crate::clipboard::IrohClipboard;
use crate::device::RuntimeDeviceManager;
use crate::media::{IrohMedia, MediaProvider, MediaRegistry};
use crate::notification::IrohNotificationChannel;
use crate::pairing::IrohPairing;
use crate::protocol::wire::{
    ControlFrame, NotificationActionActivated as WireActionActivated,
    NotificationConsumed as WireNotificationConsumed, PairingAuthenticate, control_frame,
};
use crate::protocol::{
    ClipboardMetadata, address_snapshot, decode_address_snapshot, decode_clipboard_item,
    decode_media_command, decode_media_snapshot, decode_notification_with_images,
    decode_session_hello, encode_media_command, encode_media_snapshot, read_control_frame,
    session_hello, write_control_frame,
};
use crate::storage::{AddressSource, StateStore, StoreError};
use crate::transfer::{BlobStore, StoredBlob};
use crate::{
    Clipboard, ClipboardContent, ClipboardError, ConsumedNotification, DeviceError, DeviceInfo,
    DeviceManager, FileOffer, FileShare, FileTransfer, MIST_ALPN, ManagedDevice, MediaCommand,
    MediaError, MediaEvent, MediaPlayer, MistApp, Notification, NotificationChannel,
    NotificationError, NotificationKind, Pairing, ReceivedClipboard, ReceivedNotification,
    TransferError,
};

pub(crate) use builder::MistBuilder;

/// How many sent notifications keep their action list for validation.
const MAX_TRACKED_NOTIFICATIONS: usize = 256;

/// Failure while constructing or connecting a Mist runtime.
#[derive(Debug, Error)]
pub(crate) enum MistError {
    /// The local Iroh endpoint could not be bound.
    #[error("failed to bind the Mist endpoint")]
    Bind(#[from] BindError),
    /// An outgoing connection could not be established.
    #[error("failed to connect to the Mist peer")]
    Connect(#[from] ConnectError),
    /// An incoming connection failed during its handshake.
    #[error("failed to accept the Mist peer")]
    Accept(#[from] ConnectingError),
    /// Persisted identity, trust, or address state could not be loaded.
    #[error("Mist state storage failed")]
    Storage(#[from] StoreError),
    /// The local blob store could not be opened.
    #[error("Mist file storage failed: {reason}")]
    Transfer { reason: String },
}

/// Artwork pinned in the blob store while its snapshot is being shared.
struct StoredMediaArtwork {
    mime_type: String,
    size: u64,
    hash: iroh_blobs::Hash,
    ticket: String,
    _tag: StoredBlob,
}

pub(crate) struct IncomingPairingAttempt {
    pub(crate) connection: Connection,
    pub(crate) send: SendStream,
    pub(crate) recv: RecvStream,
    pub(crate) token: [u8; 32],
}

impl std::fmt::Debug for IncomingPairingAttempt {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IncomingPairingAttempt")
            .field("remote_id", &self.connection.remote_id())
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub(crate) struct Session {
    endpoint: Endpoint,
    device_info: DeviceInfo,
    devices: DeviceRegistry,
    store: Arc<dyn StateStore>,
    local_addresses: tokio::sync::watch::Receiver<Option<PublishedAddresses>>,
    address_revisions: StdMutex<std::collections::HashMap<(EndpointId, u64), u64>>,
    notifications: broadcast::Sender<ReceivedNotification>,
    clipboard: broadcast::Sender<ReceivedClipboard>,
    activations: broadcast::Sender<crate::ActivatedNotificationAction>,
    consumed: broadcast::Sender<ConsumedNotification>,
    sent_notifications: StdMutex<std::collections::HashMap<String, Vec<String>>>,
    notification_recipients: StdMutex<std::collections::HashMap<String, Vec<EndpointId>>>,
    received_notifications: StdMutex<std::collections::HashSet<(EndpointId, String)>>,
    blob_directory: std::path::PathBuf,
    files: Arc<dyn BlobStore>,
    file_shares: StdMutex<std::collections::HashMap<String, StoredBlob>>,
    file_offers: broadcast::Sender<FileOffer>,
    received_files: broadcast::Sender<crate::ReceivedFile>,
    file_inbox_directory: Option<std::path::PathBuf>,
    pairing_attempts_tx: mpsc::Sender<IncomingPairingAttempt>,
    pairing_attempts_rx: Arc<Mutex<mpsc::Receiver<IncomingPairingAttempt>>>,
    outbox: Outbox,
    reconnecting: StdMutex<std::collections::HashSet<EndpointId>>,
    media: MediaRegistry,
    media_provider: StdMutex<Option<Arc<dyn MediaProvider>>>,
    media_command_handler: StdMutex<Option<Arc<dyn crate::MediaCommandHandler>>>,
    executor: Handle,
}

impl Session {
    fn new(
        endpoint: Endpoint,
        device_info: &DeviceInfo,
        store: Arc<dyn StateStore>,
        local_addresses: tokio::sync::watch::Receiver<Option<PublishedAddresses>>,
        blob_directory: std::path::PathBuf,
        files: Arc<dyn BlobStore>,
        file_inbox_directory: Option<std::path::PathBuf>,
    ) -> Result<Arc<Self>, StoreError> {
        let (notifications, _) = broadcast::channel(256);
        let (clipboard, _) = broadcast::channel(64);
        let (activations, _) = broadcast::channel(64);
        let (consumed, _) = broadcast::channel(64);
        let (file_offers, _) = broadcast::channel(64);
        let (received_files, _) = broadcast::channel(64);
        let (pairing_attempts_tx, pairing_attempts_rx) = mpsc::channel(32);
        let devices = DeviceRegistry::new(store.load_devices()?);
        Ok(Arc::new(Self {
            endpoint,
            device_info: device_info.clone(),
            devices,
            store,
            local_addresses,
            address_revisions: StdMutex::new(std::collections::HashMap::new()),
            notifications,
            clipboard,
            activations,
            consumed,
            sent_notifications: StdMutex::new(std::collections::HashMap::new()),
            notification_recipients: StdMutex::new(std::collections::HashMap::new()),
            received_notifications: StdMutex::new(std::collections::HashSet::new()),
            blob_directory,
            files,
            file_shares: StdMutex::new(std::collections::HashMap::new()),
            file_offers,
            received_files,
            file_inbox_directory,
            pairing_attempts_tx,
            pairing_attempts_rx: Arc::new(Mutex::new(pairing_attempts_rx)),
            outbox: Outbox::default(),
            reconnecting: StdMutex::new(std::collections::HashSet::new()),
            media: MediaRegistry::new(device_info.clone()),
            media_provider: StdMutex::new(None),
            media_command_handler: StdMutex::new(None),
            executor: Handle::current(),
        }))
    }

    fn start(self: &Arc<Self>) {
        let session = Arc::clone(self);
        self.executor
            .spawn(async move { session.accept_connections().await });
    }

    pub(crate) fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    pub(crate) fn device_info(&self) -> &DeviceInfo {
        &self.device_info
    }

    pub(crate) fn executor(&self) -> &Handle {
        &self.executor
    }

    pub(crate) fn devices(&self) -> Vec<ManagedDevice> {
        self.devices.devices()
    }

    pub(crate) fn device(&self, endpoint_id: EndpointId) -> Option<ManagedDevice> {
        self.devices.device(endpoint_id)
    }

    pub(crate) fn subscribe_devices(&self) -> broadcast::Receiver<crate::DeviceEvent> {
        self.devices.subscribe()
    }

    pub(crate) fn subscribe_notifications(&self) -> broadcast::Receiver<ReceivedNotification> {
        self.notifications.subscribe()
    }

    pub(crate) fn subscribe_clipboard(&self) -> broadcast::Receiver<ReceivedClipboard> {
        self.clipboard.subscribe()
    }

    pub(crate) fn subscribe_activations(
        &self,
    ) -> broadcast::Receiver<crate::ActivatedNotificationAction> {
        self.activations.subscribe()
    }

    pub(crate) fn subscribe_consumed(&self) -> broadcast::Receiver<ConsumedNotification> {
        self.consumed.subscribe()
    }

    /// Remembers the actions of a notification this device sent, so an
    /// activation from a peer can be validated and routed to the local app.
    fn remember_sent_notification(&self, notification: &Notification) {
        let mut sent = self
            .sent_notifications
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if sent.len() >= MAX_TRACKED_NOTIFICATIONS {
            sent.clear();
        }
        sent.insert(
            notification.id.clone(),
            notification
                .actions
                .iter()
                .map(|action| action.id.clone())
                .collect(),
        );
    }

    /// Handles an action activated on a notification this device sent.
    fn receive_notification_activation(&self, endpoint_id: EndpointId, wire: WireActionActivated) {
        let Some(actions) = self
            .sent_notifications
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&wire.notification_id)
            .cloned()
        else {
            return;
        };
        if !actions.contains(&wire.action_id) {
            return;
        }
        let _ = self.activations.send(crate::ActivatedNotificationAction {
            source_endpoint_id: endpoint_id.to_string(),
            notification_id: wire.notification_id,
            action_id: wire.action_id,
        });
    }

    /// Handles a notification consumed on either side of the bridge.
    ///
    /// A peer reports that it consumed a notification this device sent, or
    /// retracts one it sent earlier. Either way the notification is finished,
    /// so it is surfaced once and dropped from local tracking.
    fn receive_notification_consumed(
        &self,
        endpoint_id: EndpointId,
        wire: WireNotificationConsumed,
    ) {
        // Only a peer that actually received this notification may finish it.
        let owned_by_peer = self
            .notification_recipients
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&wire.notification_id)
            .is_some_and(|recipients| recipients.contains(&endpoint_id));
        let owned = owned_by_peer
            && self
                .sent_notifications
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&wire.notification_id)
                .is_some();
        let received = self
            .received_notifications
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&(endpoint_id, wire.notification_id.clone()));
        if !owned && !received {
            return;
        }
        if owned {
            self.notification_recipients
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&wire.notification_id);
        }
        let _ = self.consumed.send(ConsumedNotification {
            source_endpoint_id: endpoint_id.to_string(),
            notification_id: wire.notification_id,
        });
    }

    /// Retracts a notification this device sent from every peer that has it.
    ///
    /// Used when the local copy is dismissed. Pending copies are dropped from
    /// the outbox, and delivered ones are removed on the peers.
    pub(crate) async fn retract_notification(
        self: &Arc<Self>,
        notification_id: String,
    ) -> Result<(), NotificationError> {
        let recipients = self
            .notification_recipients
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&notification_id)
            .unwrap_or_default();
        self.sent_notifications
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&notification_id);
        for endpoint_id in recipients {
            self.outbox
                .cancel_notification(endpoint_id, &notification_id);
            let Some(connection) = self.devices.connection(endpoint_id) else {
                continue;
            };
            let Ok(mut stream) = connection.open_uni().await else {
                continue;
            };
            let frame = ControlFrame {
                body: Some(control_frame::Body::NotificationConsumed(
                    WireNotificationConsumed {
                        notification_id: notification_id.clone(),
                    },
                )),
            };
            let _ = write_control_frame(&mut stream, &frame).await;
            let _ = stream.finish();
        }
        Ok(())
    }

    /// Sends an activated action back to the device that owns the notification.
    pub(crate) async fn activate_notification(
        self: &Arc<Self>,
        endpoint_id: EndpointId,
        notification_id: String,
        action_id: String,
    ) -> Result<(), NotificationError> {
        if !self.devices.contains(endpoint_id) {
            return Err(NotificationError::UnknownDevice {
                endpoint_id: endpoint_id.to_string(),
            });
        }
        let connection = self.devices.connection(endpoint_id).ok_or_else(|| {
            NotificationError::Disconnected {
                endpoint_id: endpoint_id.to_string(),
            }
        })?;
        let mut stream =
            connection
                .open_uni()
                .await
                .map_err(|error| NotificationError::Transport {
                    reason: error.to_string(),
                })?;
        write_control_frame(
            &mut stream,
            &ControlFrame {
                body: Some(control_frame::Body::NotificationActionActivated(
                    WireActionActivated {
                        notification_id,
                        action_id,
                    },
                )),
            },
        )
        .await
        .map_err(|error| NotificationError::Transport {
            reason: error.to_string(),
        })?;
        stream
            .finish()
            .map_err(|error| NotificationError::Transport {
                reason: error.to_string(),
            })
    }

    /// Reports that a notification sent by `endpoint_id` is no longer needed.
    ///
    /// Undelivered copies are dropped from the outbox; delivered ones are
    /// forwarded so the owner can clear its own copy.
    pub(crate) async fn consume_notification(
        self: &Arc<Self>,
        endpoint_id: EndpointId,
        notification_id: String,
    ) -> Result<(), NotificationError> {
        if !self.devices.contains(endpoint_id) {
            return Err(NotificationError::UnknownDevice {
                endpoint_id: endpoint_id.to_string(),
            });
        }
        self.outbox
            .cancel_notification(endpoint_id, &notification_id);
        let Some(connection) = self.devices.connection(endpoint_id) else {
            // Nothing was delivered, and the pending copy is already dropped.
            return Ok(());
        };
        let mut stream =
            connection
                .open_uni()
                .await
                .map_err(|error| NotificationError::Transport {
                    reason: error.to_string(),
                })?;
        write_control_frame(
            &mut stream,
            &ControlFrame {
                body: Some(control_frame::Body::NotificationConsumed(
                    WireNotificationConsumed { notification_id },
                )),
            },
        )
        .await
        .map_err(|error| NotificationError::Transport {
            reason: error.to_string(),
        })?;
        stream
            .finish()
            .map_err(|error| NotificationError::Transport {
                reason: error.to_string(),
            })
    }

    pub(crate) fn subscribe_media(&self) -> broadcast::Receiver<MediaEvent> {
        self.media.subscribe()
    }

    pub fn media_players(&self) -> Vec<MediaPlayer> {
        self.media.list()
    }

    /// Attaches the platform media backend and starts watching it.
    pub fn attach_media(self: &Arc<Self>, provider: &Arc<dyn MediaProvider>) {
        let session = Arc::clone(self);
        *self
            .media_provider
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Arc::clone(provider));
        let Some(watcher) = provider.watch().ok() else {
            return;
        };
        self.publish_media_snapshot();
        self.executor.spawn_blocking(move || {
            let mut watcher = watcher;
            loop {
                match watcher.next() {
                    Ok(Some(players)) => {
                        session.media.set_local(&players);
                        session.publish_media_snapshot();
                    }
                    Ok(None) | Err(_) => return,
                }
            }
        });
    }

    /// Sends the current local player snapshot to one peer.
    fn send_media_snapshot_to(self: &Arc<Self>, connection: Connection) {
        let players = self.media.list();
        let endpoint_id = connection.remote_id();
        let session = Arc::clone(self);
        self.executor.spawn(async move {
            let snapshot = session.encode_snapshot(&players, &[endpoint_id]).await;
            let Ok(mut stream) = connection.open_uni().await else {
                return;
            };
            let frame = ControlFrame {
                body: Some(control_frame::Body::MediaSnapshot(snapshot)),
            };
            if write_control_frame(&mut stream, &frame).await.is_ok() {
                let _ = stream.finish();
            }
        });
    }

    /// Records app-provided local players and broadcasts them to peers.
    pub(crate) fn publish_local_media(self: &Arc<Self>, players: Vec<MediaPlayer>) {
        let local = players
            .into_iter()
            .filter(|player| player.origin == crate::MediaOrigin::Local)
            .map(|player| crate::media::LocalPlayer {
                id: player.id,
                name: player.name,
                status: player.status,
                metadata: player.metadata,
                commands: player.capabilities.commands,
                position_ms: player.position_ms,
            })
            .collect::<Vec<_>>();
        self.media.set_local(&local);
        self.publish_media_snapshot();
    }

    /// Registers a platform handler for inbound commands. Used by hosts that
    /// expose the runtime through generated bindings instead of a Rust provider.
    pub fn set_media_command_handler(
        self: &Arc<Self>,
        handler: Arc<dyn crate::MediaCommandHandler>,
    ) {
        *self
            .media_command_handler
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(handler);
    }

    /// Broadcasts the local player snapshot to every connected peer.
    fn publish_media_snapshot(self: &Arc<Self>) {
        let players = self.media.list();
        let connections = self.devices.selected_connections();
        let session = Arc::clone(self);
        for (endpoint_id, connection) in connections {
            let Some(connection) = connection else {
                continue;
            };
            let session = Arc::clone(&session);
            let players = players.clone();
            self.executor.spawn(async move {
                let snapshot = session.encode_snapshot(&players, &[endpoint_id]).await;
                let Ok(mut stream) = connection.open_uni().await else {
                    return;
                };
                let frame = ControlFrame {
                    body: Some(control_frame::Body::MediaSnapshot(snapshot)),
                };
                if write_control_frame(&mut stream, &frame).await.is_ok() {
                    let _ = stream.finish();
                }
            });
        }
    }

    /// Imports local artwork and encodes a snapshot, authorizing artwork for `peers`.
    async fn encode_snapshot(
        &self,
        players: &[crate::MediaPlayer],
        peers: &[EndpointId],
    ) -> crate::protocol::wire::MediaSnapshot {
        let mut entries = Vec::with_capacity(players.len());
        let mut artwork = Vec::with_capacity(players.len());
        for player in players {
            let stored = self.import_media_artwork(player).await;
            if let Some(stored) = &stored {
                for peer in peers {
                    self.files.authorize(*peer, stored.hash);
                }
            }
            artwork.push(stored);
        }
        for (player, stored) in players.iter().zip(artwork) {
            entries.push(crate::protocol::MediaSnapshotEntry {
                player,
                artwork: stored
                    .as_ref()
                    .map(|stored| crate::protocol::MediaArtworkWire {
                        mime_type: stored.mime_type.clone(),
                        size: stored.size,
                        hash: stored.hash,
                        ticket: stored.ticket.clone(),
                    }),
            });
        }
        encode_media_snapshot(&entries)
    }

    /// Imports a player's local artwork into the blob store.
    ///
    /// Only local files are imported. Remote `art_url` values are passed through
    /// to peers as-is, since fetching arbitrary URLs is a platform concern.
    async fn import_media_artwork(
        &self,
        player: &crate::MediaPlayer,
    ) -> Option<StoredMediaArtwork> {
        let artwork = player.metadata.artwork.as_ref()?;
        let mime_type = artwork.mime_type.clone();
        let path = std::path::PathBuf::from(&artwork.path);
        if !mime_type.starts_with("image/") {
            return None;
        }
        let stored = self.files.import_file(path.clone()).await.ok()?;
        let size = std::fs::metadata(&path).ok()?.len();
        let ticket = self.files.ticket(stored.hash).to_string();
        Some(StoredMediaArtwork {
            mime_type,
            size,
            hash: stored.hash,
            ticket,
            _tag: stored,
        })
    }

    /// Routes a transport command to a remote player.
    pub(crate) async fn send_media_command(
        self: &Arc<Self>,
        player_id: String,
        command: MediaCommand,
    ) -> Result<(), MediaError> {
        let player = self
            .media
            .list()
            .into_iter()
            .find(|player| player.id == player_id)
            .ok_or_else(|| MediaError::UnknownPlayer {
                player_id: player_id.clone(),
            })?;
        match player.origin {
            crate::MediaOrigin::Local => {
                let provider = self
                    .media_provider
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone()
                    .ok_or_else(|| MediaError::Transport {
                        reason: "no local media backend".to_owned(),
                    })?;
                provider
                    .command(&player_id, command)
                    .await
                    .map_err(|error| MediaError::Transport {
                        reason: error.to_string(),
                    })
            }
            crate::MediaOrigin::Remote => {
                let endpoint_id = EndpointId::from_str(&player.endpoint_id).map_err(|error| {
                    MediaError::Transport {
                        reason: error.to_string(),
                    }
                })?;
                let connection =
                    self.devices
                        .connection(endpoint_id)
                        .ok_or_else(|| MediaError::Transport {
                            reason: format!("device {endpoint_id} is disconnected"),
                        })?;
                let mut stream =
                    connection
                        .open_uni()
                        .await
                        .map_err(|error| MediaError::Transport {
                            reason: error.to_string(),
                        })?;
                write_control_frame(
                    &mut stream,
                    &ControlFrame {
                        body: Some(control_frame::Body::MediaCommand(encode_media_command(
                            &player_id, command,
                        ))),
                    },
                )
                .await
                .map_err(|error| MediaError::Transport {
                    reason: error.to_string(),
                })?;
                stream.finish().map_err(|error| MediaError::Transport {
                    reason: error.to_string(),
                })
            }
        }
    }

    pub(crate) async fn next_pairing_attempt(&self) -> Option<IncomingPairingAttempt> {
        self.pairing_attempts_rx.lock().await.recv().await
    }

    pub(crate) async fn connect_candidate(
        &self,
        peer: EndpointAddr,
    ) -> Result<Connection, MistError> {
        let expected_id = peer.id;
        let connection = self.endpoint.connect(peer, MIST_ALPN).await?;
        debug_assert_eq!(connection.remote_id(), expected_id);
        Ok(connection)
    }

    pub(crate) fn subscribe_files(&self) -> broadcast::Receiver<FileOffer> {
        self.file_offers.subscribe()
    }

    pub(crate) fn subscribe_inbox(&self) -> broadcast::Receiver<crate::ReceivedFile> {
        self.received_files.subscribe()
    }

    pub(crate) async fn share_file(
        self: &Arc<Self>,
        path: String,
        targets: Vec<String>,
    ) -> Result<FileShare, TransferError> {
        let stored = self
            .files
            .import_file(std::path::PathBuf::from(&path))
            .await?;
        let name = std::path::Path::new(&path)
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(TransferError::Unreadable {
                path: path.clone(),
                reason: "the path has no file name".to_owned(),
            })?
            .to_owned();
        let size = std::fs::metadata(&path)
            .map_err(|error| TransferError::Unreadable {
                path: path.clone(),
                reason: error.to_string(),
            })?
            .len();
        let id = stored.hash.to_hex();
        let ticket = iroh_blobs::ticket::BlobTicket::new(
            self.endpoint.addr(),
            stored.hash,
            iroh_blobs::BlobFormat::Raw,
        )
        .to_string();
        let targets = self.resolve_file_targets(targets)?;
        for endpoint_id in targets {
            self.files.authorize(endpoint_id, stored.hash);
            self.schedule(
                endpoint_id,
                Outbound::FileOffer(FileShare {
                    id: id.clone(),
                    name: name.clone(),
                    size,
                    ticket: ticket.clone(),
                }),
            );
        }
        let share = FileShare {
            id: id.clone(),
            name,
            size,
            ticket,
        };
        self.file_shares
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, stored);
        Ok(share)
    }

    pub(crate) async fn accept_file(
        self: &Arc<Self>,
        offer: FileOffer,
    ) -> Result<String, TransferError> {
        let session = Arc::clone(self);
        let executor = self.executor.clone();
        executor
            .spawn(async move { session.accept_file_inner(offer).await })
            .await
            .map_err(|error| TransferError::Transport {
                reason: error.to_string(),
            })?
    }

    async fn accept_file_inner(&self, offer: FileOffer) -> Result<String, TransferError> {
        let endpoint_id = EndpointId::from_str(&offer.source_endpoint_id).map_err(|error| {
            TransferError::InvalidEndpointId {
                reason: error.to_string(),
            }
        })?;
        if !self.devices.contains(endpoint_id) {
            return Err(TransferError::UntrustedSource);
        }
        let ticket = iroh_blobs::ticket::BlobTicket::from_str(&offer.ticket).map_err(|_| {
            TransferError::Transport {
                reason: "invalid file ticket".to_owned(),
            }
        })?;
        if ticket.addr().id != endpoint_id {
            return Err(TransferError::UntrustedSource);
        }
        let destination = self
            .file_inbox_directory
            .clone()
            .ok_or(TransferError::InboxNotConfigured)?;
        self.files.download(endpoint_id, ticket.hash()).await?;
        let file_name = available_file_name(&destination, &offer.name)?;
        let path = self
            .files
            .export(ticket.hash(), destination, &file_name)
            .await?;
        let source_device = self
            .devices
            .device_info(endpoint_id)
            .unwrap_or_else(|| offer.source_device.clone());
        let _ = self.received_files.send(crate::ReceivedFile {
            source_endpoint_id: endpoint_id.to_string(),
            source_device,
            name: offer.name,
            path: path.to_string_lossy().into_owned(),
            size: std::fs::metadata(&path).map_or(0, |metadata| metadata.len()),
        });
        Ok(path.to_string_lossy().into_owned())
    }

    fn resolve_file_targets(&self, targets: Vec<String>) -> Result<Vec<EndpointId>, TransferError> {
        if targets.is_empty() {
            let selected = self
                .devices
                .selected_connections()
                .into_iter()
                .map(|(endpoint_id, _)| endpoint_id)
                .collect::<Vec<_>>();
            if selected.is_empty() {
                return Err(TransferError::NoSelectedDevices);
            }
            return Ok(selected);
        }
        targets
            .into_iter()
            .map(|target| {
                let endpoint_id = EndpointId::from_str(&target).map_err(|error| {
                    TransferError::InvalidEndpointId {
                        reason: error.to_string(),
                    }
                })?;
                if !self.devices.contains(endpoint_id) {
                    return Err(TransferError::UnknownDevice {
                        endpoint_id: target,
                    });
                }
                Ok(endpoint_id)
            })
            .collect()
    }

    pub(crate) fn install(
        self: &Arc<Self>,
        connection: Connection,
        device: DeviceInfo,
        addresses: &[iroh::TransportAddr],
    ) -> Result<Connection, StoreError> {
        let endpoint_id = connection.remote_id();
        self.store.upsert_device(endpoint_id, &device, true)?;
        self.sync_addresses(endpoint_id, AddressSource::AuthenticatedPeer, addresses)?;
        let generation = self
            .devices
            .install(endpoint_id, device, connection.clone());
        self.start_connection_worker(connection.clone(), generation);
        self.start_address_publisher(connection.clone());
        self.start_outbox_flush(connection.clone());
        self.send_media_snapshot_to(connection.clone());
        Ok(connection)
    }

    pub(crate) fn add_pairing_addresses(
        &self,
        endpoint_id: EndpointId,
        addresses: &[iroh::TransportAddr],
    ) -> Result<(), StoreError> {
        self.store
            .add_addresses(endpoint_id, AddressSource::PairingOffer, addresses)
    }

    fn sync_addresses(
        &self,
        endpoint_id: EndpointId,
        source: AddressSource,
        addresses: &[iroh::TransportAddr],
    ) -> Result<(), StoreError> {
        use std::collections::BTreeSet;

        let previous = self
            .store
            .load_addresses_from(endpoint_id, source)?
            .into_iter()
            .collect::<BTreeSet<_>>();
        let current = addresses.iter().cloned().collect::<BTreeSet<_>>();
        let added = current.difference(&previous).cloned().collect::<Vec<_>>();
        let removed = previous.difference(&current).cloned().collect::<Vec<_>>();
        self.store.add_addresses(endpoint_id, source, &added)?;
        self.store.remove_addresses(endpoint_id, source, &removed)
    }

    fn start_address_publisher(self: &Arc<Self>, connection: Connection) {
        let mut updates = self.local_addresses.clone();
        self.executor.spawn(async move {
            loop {
                let update = updates.borrow_and_update().clone();
                if let Some(update) = update {
                    let Ok(mut stream) = connection.open_uni().await else {
                        return;
                    };
                    let frame = ControlFrame {
                        body: Some(control_frame::Body::AddressSnapshot(address_snapshot(
                            update.revision,
                            update.addresses,
                        ))),
                    };
                    if write_control_frame(&mut stream, &frame).await.is_err()
                        || stream.finish().is_err()
                    {
                        return;
                    }
                }
                tokio::select! {
                    _ = connection.closed() => return,
                    changed = updates.changed() => {
                        if changed.is_err() {
                            return;
                        }
                    }
                }
            }
        });
    }

    fn start_connection_worker(self: &Arc<Self>, connection: Connection, generation: u64) {
        let session = Arc::clone(self);
        self.executor.spawn(async move {
            let endpoint_id = connection.remote_id();
            let mut path_events = connection.path_events();
            loop {
                tokio::select! {
                    _ = connection.closed() => break,
                    event = path_events.next() => {
                        if event.is_none() {
                            break;
                        }
                        session.devices.notify_changed(endpoint_id, generation);
                    }
                    incoming = connection.accept_uni() => {
                        let Ok(stream) = incoming else { break };
                        let session = Arc::clone(&session);
                        let executor = session.executor.clone();
                        executor.spawn(async move {
                            session.receive_feature_frame(endpoint_id, generation, stream).await;
                        });
                    }
                }
            }
            session
                .address_revisions
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&(endpoint_id, generation));
            session.devices.mark_disconnected(endpoint_id, generation);
            session.media.remove_remote(&endpoint_id.to_string());
        });
    }

    #[allow(
        clippy::too_many_lines,
        reason = "control frames share one dispatch loop"
    )]
    async fn receive_feature_frame(
        self: &Arc<Self>,
        endpoint_id: EndpointId,
        generation: u64,
        mut stream: RecvStream,
    ) {
        let Ok(frame) = read_control_frame(&mut stream).await else {
            return;
        };
        match frame.body {
            Some(control_frame::Body::Notification(wire)) => {
                let Ok(decoded) = decode_notification_with_images(wire) else {
                    return;
                };
                let notification = self.receive_notification_images(endpoint_id, decoded).await;
                let Some(source_device) = self.devices.device_info(endpoint_id) else {
                    return;
                };
                {
                    let mut received = self
                        .received_notifications
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner);
                    if received.len() >= MAX_TRACKED_NOTIFICATIONS * 4 {
                        received.clear();
                    }
                    received.insert((endpoint_id, notification.id.clone()));
                }
                let _ = self.notifications.send(ReceivedNotification {
                    source_endpoint_id: endpoint_id.to_string(),
                    source_device,
                    notification,
                });
            }
            Some(control_frame::Body::ClipboardItem(wire)) => {
                let Ok(metadata) = decode_clipboard_item(wire) else {
                    return;
                };
                let Some(source_device) = self.devices.device_info(endpoint_id) else {
                    return;
                };
                let (content, content_id) = match metadata {
                    ClipboardMetadata::Text { text } => {
                        let fingerprint = crate::runtime::outbox::text_fingerprint(&text);
                        (ClipboardContent::Text { text }, fingerprint)
                    }
                    ClipboardMetadata::Blob { mime_type, ticket } => {
                        let hash = ticket.hash();
                        let Ok(()) = self.files.download(endpoint_id, hash).await else {
                            return;
                        };
                        let Ok(path) = self
                            .files
                            .export(
                                hash,
                                self.blob_directory.clone(),
                                &format!("{}.blob", hash.to_hex()),
                            )
                            .await
                        else {
                            return;
                        };
                        (
                            ClipboardContent::Blob {
                                mime_type,
                                path: path.to_string_lossy().into_owned(),
                            },
                            *hash.as_bytes(),
                        )
                    }
                };
                self.outbox.remember_received(content_id);
                let _ = self.clipboard.send(ReceivedClipboard {
                    source_endpoint_id: endpoint_id.to_string(),
                    source_device,
                    content,
                    sha256: hex(&content_id),
                });
            }
            Some(control_frame::Body::NotificationActionActivated(wire)) => {
                self.receive_notification_activation(endpoint_id, wire);
            }
            Some(control_frame::Body::NotificationConsumed(wire)) => {
                self.receive_notification_consumed(endpoint_id, wire);
            }
            Some(control_frame::Body::FileOffer(wire)) => {
                let Ok(offer) = crate::protocol::decode_file_offer(wire) else {
                    return;
                };
                let Ok(ticket) = iroh_blobs::ticket::BlobTicket::from_str(&offer.ticket) else {
                    return;
                };
                if ticket.addr().id != endpoint_id {
                    return;
                }
                let Some(source_device) = self.devices.device_info(endpoint_id) else {
                    return;
                };
                let _ = self.file_offers.send(FileOffer {
                    source_endpoint_id: endpoint_id.to_string(),
                    source_device,
                    id: offer.id,
                    name: offer.name,
                    size: offer.size,
                    ticket: offer.ticket,
                });
            }
            Some(control_frame::Body::MediaSnapshot(snapshot)) => {
                Arc::clone(self).receive_media_snapshot(endpoint_id, snapshot);
            }
            Some(control_frame::Body::MediaCommand(request)) => {
                self.receive_media_command(request).await;
            }
            Some(control_frame::Body::AddressSnapshot(snapshot)) => {
                if !self.accept_address_revision(endpoint_id, generation, snapshot.revision) {
                    return;
                }
                let Ok(addresses) = decode_address_snapshot(snapshot) else {
                    return;
                };
                let _ =
                    self.sync_addresses(endpoint_id, AddressSource::AuthenticatedPeer, &addresses);
            }
            _ => {}
        }
    }

    /// Streams notification artwork into app-private files.
    async fn receive_notification_images(
        &self,
        endpoint_id: EndpointId,
        decoded: crate::protocol::DecodedNotification,
    ) -> crate::Notification {
        let crate::protocol::DecodedNotification {
            mut notification,
            images,
        } = decoded;
        let mut images = images.into_iter();
        for target in [
            notification
                .app_icon
                .is_some()
                .then_some(&mut notification.app_icon),
            notification
                .image
                .is_some()
                .then_some(&mut notification.image),
        ] {
            let Some(slot) = target else {
                continue;
            };
            let Some(metadata) = images.next() else {
                continue;
            };
            if let Ok(()) = self.files.download(endpoint_id, metadata.hash()).await
                && let Ok(path) = self
                    .files
                    .export(
                        metadata.hash(),
                        self.blob_directory.clone(),
                        &format!("{}.blob", metadata.hash().to_hex()),
                    )
                    .await
                && let Some(image) = slot.as_mut()
            {
                image.path = path.to_string_lossy().into_owned();
            }
        }
        notification
    }

    fn receive_media_snapshot(
        self: &Arc<Self>,
        endpoint_id: EndpointId,
        snapshot: crate::protocol::wire::MediaSnapshot,
    ) {
        let Ok(decoded) = decode_media_snapshot(snapshot) else {
            return;
        };
        let Some(device) = self.devices.device_info(endpoint_id) else {
            return;
        };
        let session = Arc::clone(self);
        self.executor.spawn(async move {
            let mut players = Vec::with_capacity(decoded.len());
            for state in &decoded {
                let mut player =
                    crate::media::remote_player(&endpoint_id.to_string(), &device, state);
                player.artwork_path = session
                    .receive_media_artwork(endpoint_id, state.artwork.as_ref())
                    .await;
                players.push(player);
            }
            session
                .media
                .set_remote(&endpoint_id.to_string(), &device, &players);
        });
    }

    /// Downloads one peer's artwork blob into the local blob directory.
    async fn receive_media_artwork(
        &self,
        endpoint_id: EndpointId,
        artwork: Option<&crate::protocol::MediaArtworkRef>,
    ) -> Option<String> {
        let artwork = artwork?;
        let hash = artwork.ticket.hash();
        self.files.download(endpoint_id, hash).await.ok()?;
        let extension = match artwork.mime_type.as_str() {
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/webp" => "webp",
            "image/gif" => "gif",
            _ => "img",
        };
        let path = self
            .files
            .export(
                hash,
                self.blob_directory.clone(),
                &format!("media-{}.{extension}", hash.to_hex()),
            )
            .await
            .ok()?;
        Some(path.to_string_lossy().into_owned())
    }

    async fn receive_media_command(&self, request: crate::protocol::wire::MediaCommandRequest) {
        let Ok((player_id, command)) = decode_media_command(request) else {
            return;
        };
        let provider = self
            .media_provider
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(provider) = provider {
            let _ = provider.command(&player_id, command).await;
            return;
        }
        let handler = self
            .media_command_handler
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(handler) = handler {
            let _ = handler.command(player_id, command);
        }
    }

    fn accept_address_revision(
        &self,
        endpoint_id: EndpointId,
        generation: u64,
        revision: u64,
    ) -> bool {
        let mut revisions = self
            .address_revisions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let current = revisions.entry((endpoint_id, generation)).or_insert(0);
        if revision <= *current {
            false
        } else {
            *current = revision;
            true
        }
    }

    async fn accept_connections(self: Arc<Self>) {
        while let Some(incoming) = self.endpoint.accept().await {
            let session = Arc::clone(&self);
            self.executor.spawn(async move {
                let Ok(connection) = incoming.await else {
                    return;
                };
                if connection.alpn() == iroh_blobs::ALPN {
                    session.files.serve(connection).await;
                } else {
                    session.route_connection(connection).await;
                }
            });
        }
    }

    async fn route_connection(self: Arc<Self>, connection: Connection) {
        let Ok((mut send, mut recv)) = connection.accept_bi().await else {
            return;
        };
        let Ok(frame) = read_control_frame(&mut recv).await else {
            return;
        };
        match frame.body {
            Some(control_frame::Body::PairingAuthenticate(PairingAuthenticate {
                pairing_token,
            })) => {
                let Ok(token) = pairing_token.as_slice().try_into() else {
                    return;
                };
                let _ = self
                    .pairing_attempts_tx
                    .send(IncomingPairingAttempt {
                        connection,
                        send,
                        recv,
                        token,
                    })
                    .await;
            }
            Some(control_frame::Body::SessionHello(hello)) => {
                let endpoint_id = connection.remote_id();
                if !self.devices.contains(endpoint_id) {
                    connection.close(2_u32.into(), b"unpaired endpoint");
                    return;
                }
                let Ok(peer) = decode_session_hello(hello) else {
                    return;
                };
                let response = ControlFrame {
                    body: Some(control_frame::Body::SessionHello(session_hello(
                        self.device_info(),
                        self.endpoint.addr(),
                    ))),
                };
                if write_control_frame(&mut send, &response).await.is_err() {
                    return;
                }
                if send.finish().is_err() {
                    return;
                }
                let _ = self.install(connection, peer.device, &peer.addresses);
            }
            _ => connection.close(2_u32.into(), b"unexpected session handshake"),
        }
    }

    pub(crate) async fn connect_device(
        self: &Arc<Self>,
        endpoint_id: EndpointId,
    ) -> Result<(), DeviceError> {
        let _known =
            self.devices
                .device_info(endpoint_id)
                .ok_or_else(|| DeviceError::UnknownDevice {
                    endpoint_id: endpoint_id.to_string(),
                })?;
        if self.devices.connection(endpoint_id).is_some() {
            return Ok(());
        }
        self.devices.set_connecting(endpoint_id)?;
        let result = async {
            let connection = self
                .endpoint
                .connect(endpoint_id, MIST_ALPN)
                .await
                .map_err(|error| DeviceError::Connect {
                    reason: error.to_string(),
                })?;
            let (mut send, mut recv) =
                connection
                    .open_bi()
                    .await
                    .map_err(|error| DeviceError::Protocol {
                        reason: error.to_string(),
                    })?;
            write_control_frame(
                &mut send,
                &ControlFrame {
                    body: Some(control_frame::Body::SessionHello(session_hello(
                        self.device_info(),
                        self.endpoint.addr(),
                    ))),
                },
            )
            .await
            .map_err(|error| DeviceError::Protocol {
                reason: error.to_string(),
            })?;
            let response =
                read_control_frame(&mut recv)
                    .await
                    .map_err(|error| DeviceError::Protocol {
                        reason: error.to_string(),
                    })?;
            let Some(control_frame::Body::SessionHello(hello)) = response.body else {
                return Err(DeviceError::Protocol {
                    reason: "peer did not return a session hello".to_owned(),
                });
            };
            let peer = decode_session_hello(hello).map_err(|error| DeviceError::Protocol {
                reason: error.to_string(),
            })?;
            send.finish().map_err(|error| DeviceError::Protocol {
                reason: error.to_string(),
            })?;
            self.install(connection, peer.device, &peer.addresses)
                .map_err(|error| DeviceError::Storage {
                    reason: error.to_string(),
                })?;
            Ok(())
        }
        .await;
        if result.is_err() {
            self.devices.clear_connecting(endpoint_id);
        }
        result
    }

    pub(crate) fn select_device(
        &self,
        endpoint_id: EndpointId,
        selected: bool,
    ) -> Result<(), DeviceError> {
        if !self.devices.contains(endpoint_id) {
            return Err(DeviceError::UnknownDevice {
                endpoint_id: endpoint_id.to_string(),
            });
        }
        self.store
            .set_selected(endpoint_id, selected)
            .map_err(|error| DeviceError::Storage {
                reason: error.to_string(),
            })?;
        self.devices.select(endpoint_id, selected)
    }

    pub(crate) fn disconnect_device(&self, endpoint_id: EndpointId) -> Result<(), DeviceError> {
        self.devices.disconnect(endpoint_id)
    }

    pub(crate) fn forget_device(&self, endpoint_id: EndpointId) -> Result<(), DeviceError> {
        if !self.devices.contains(endpoint_id) {
            return Err(DeviceError::UnknownDevice {
                endpoint_id: endpoint_id.to_string(),
            });
        }
        self.store
            .forget_device(endpoint_id)
            .map_err(|error| DeviceError::Storage {
                reason: error.to_string(),
            })?;
        self.devices.forget(endpoint_id)
    }

    pub(crate) async fn push_notification(
        self: &Arc<Self>,
        notification: &Notification,
        targets: Vec<String>,
    ) -> Result<(), NotificationError> {
        // Media transport state belongs to the media feature; the generic
        // notification bridge must not flood peers with track updates.
        if notification.kind == NotificationKind::Media {
            return Ok(());
        }

        let targets = self.resolve_targets(targets).map_err(|error| match error {
            TargetError::Invalid(reason) => NotificationError::InvalidEndpointId { reason },
            TargetError::Unknown(endpoint_id) => NotificationError::UnknownDevice { endpoint_id },
            TargetError::NoSelected => NotificationError::NoSelectedDevices,
        })?;

        self.remember_sent_notification(notification);
        let mut images = Vec::new();
        for (image, limit, label) in [
            (
                notification.app_icon.as_ref(),
                crate::protocol::MAX_NOTIFICATION_ICON_BYTES,
                "icon",
            ),
            (
                notification.image.as_ref(),
                crate::protocol::MAX_NOTIFICATION_IMAGE_BYTES,
                "image",
            ),
        ] {
            let Some(image) = image else {
                images.push(None);
                continue;
            };
            let attachment = self
                .prepare_attachment(&image.mime_type, &image.path, limit)
                .await
                .map_err(|error| NotificationError::Transport {
                    reason: format!("{label}: {error}"),
                })?;
            images.push(Some(attachment));
        }
        {
            let mut recipients = self
                .notification_recipients
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if recipients.len() >= MAX_TRACKED_NOTIFICATIONS {
                recipients.clear();
            }
            recipients.insert(notification.id.clone(), targets.clone());
        }
        for endpoint_id in targets {
            for image in images.iter().flatten() {
                self.files.authorize(endpoint_id, image.hash);
            }
            self.schedule(
                endpoint_id,
                Outbound::Notification {
                    notification: notification.clone(),
                    images: images.clone(),
                },
            );
        }
        Ok(())
    }

    pub(crate) async fn push_clipboard(
        self: &Arc<Self>,
        content: &ClipboardContent,
        targets: Vec<String>,
    ) -> Result<(), ClipboardError> {
        let blob = match content {
            ClipboardContent::Text { text } => {
                if text.is_empty() {
                    return Err(ClipboardError::Empty);
                }
                None
            }
            ClipboardContent::Blob { mime_type, path } => Some(
                self.prepare_attachment(mime_type, path, crate::protocol::MAX_CLIPBOARD_BLOB_BYTES)
                    .await
                    .map_err(|error| ClipboardError::Unreadable {
                        reason: error.to_string(),
                    })?,
            ),
        };

        let targets = self.resolve_targets(targets).map_err(|error| match error {
            TargetError::Invalid(reason) => ClipboardError::InvalidEndpointId { reason },
            TargetError::Unknown(endpoint_id) => ClipboardError::UnknownDevice { endpoint_id },
            TargetError::NoSelected => ClipboardError::NoSelectedDevices,
        })?;

        for endpoint_id in targets {
            if let Some(blob) = blob.as_ref() {
                self.files.authorize(endpoint_id, blob.hash);
            }
            self.schedule(
                endpoint_id,
                Outbound::Clipboard {
                    content: content.clone(),
                    blob: blob.clone(),
                },
            );
        }
        Ok(())
    }

    async fn prepare_attachment(
        &self,
        mime_type: &str,
        path: &str,
        limit: u64,
    ) -> Result<crate::runtime::outbox::BlobAttachment, TransferError> {
        crate::protocol::validate_mime_type(mime_type).map_err(|error| {
            TransferError::Transport {
                reason: error.to_string(),
            }
        })?;
        let stored = self
            .files
            .import_file(std::path::PathBuf::from(path))
            .await?;
        let size = std::fs::metadata(path)
            .map_err(|error| TransferError::Unreadable {
                path: path.to_owned(),
                reason: error.to_string(),
            })?
            .len();
        if size > limit {
            return Err(TransferError::Transport {
                reason: format!("attachment exceeds {limit} bytes"),
            });
        }
        Ok(crate::runtime::outbox::BlobAttachment {
            mime_type: mime_type.to_owned(),
            size,
            hash: stored.hash,
            ticket: self.files.ticket(stored.hash),
        })
    }

    /// Resolves explicit targets, or every selected device when none are given.
    fn resolve_targets(&self, targets: Vec<String>) -> Result<Vec<EndpointId>, TargetError> {
        if targets.is_empty() {
            let selected = self
                .devices
                .selected_connections()
                .into_iter()
                .map(|(endpoint_id, _)| endpoint_id)
                .collect::<Vec<_>>();
            if selected.is_empty() {
                return Err(TargetError::NoSelected);
            }
            return Ok(selected);
        }

        targets
            .into_iter()
            .map(|target| {
                let endpoint_id = EndpointId::from_str(&target)
                    .map_err(|error| TargetError::Invalid(error.to_string()))?;
                if !self.devices.contains(endpoint_id) {
                    return Err(TargetError::Unknown(target));
                }
                Ok(endpoint_id)
            })
            .collect()
    }

    /// Sends now when connected, otherwise queues and starts a reconnect.
    ///
    /// Content that arrived from a peer is suppressed here so it is not echoed
    /// back; features never have to reason about this.
    fn schedule(self: &Arc<Self>, endpoint_id: EndpointId, message: Outbound) -> bool {
        if let Some(fingerprint) = message.fingerprint()
            && self.outbox.suppress_echo(fingerprint)
        {
            return false;
        }
        let connection = self.devices.connection(endpoint_id);
        self.outbox.push(endpoint_id, message);
        match connection {
            Some(connection) => self.start_outbox_flush(connection),
            None => self.ensure_connected(endpoint_id),
        }
        true
    }

    /// Starts a reconnect unless one is already in flight.
    fn ensure_connected(self: &Arc<Self>, endpoint_id: EndpointId) {
        let inserted = self
            .reconnecting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(endpoint_id);
        if !inserted {
            return;
        }
        let session = Arc::clone(self);
        self.executor.spawn(async move {
            let _ = session.connect_device(endpoint_id).await;
            session
                .reconnecting
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&endpoint_id);
        });
    }

    /// Drains the endpoint's outbox in order while the connection lives.
    fn start_outbox_flush(self: &Arc<Self>, connection: Connection) {
        let endpoint_id = connection.remote_id();
        if !self.outbox.begin_flush(endpoint_id) {
            return;
        }
        let session = Arc::clone(self);
        self.executor.spawn(async move {
            loop {
                let Some(message) = session.outbox.pop(endpoint_id) else {
                    break;
                };
                if let Err(reason) = outbox::send(&connection, &message).await {
                    debug_assert!(!reason.is_empty());
                    session.outbox.push_front(endpoint_id, message);
                    break;
                }
            }
            session.outbox.end_flush(endpoint_id);
            if session.outbox.has_pending(endpoint_id) {
                session.ensure_connected(endpoint_id);
            }
        });
    }

    async fn close(&self) {
        self.devices.close_all();
        let _ = self.files.shutdown().await;
        self.endpoint.close().await;
    }
}

/// Failure while resolving outbound message targets.
#[derive(Debug)]
enum TargetError {
    Invalid(String),
    Unknown(String),
    NoSelected,
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

fn available_file_name(
    directory: &std::path::Path,
    requested: &str,
) -> Result<String, TransferError> {
    let candidate = std::path::Path::new(requested);
    let name = candidate
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| *name != "." && *name != "..")
        .ok_or(TransferError::Transport {
            reason: "offered file name is invalid".to_owned(),
        })?;
    if !directory.join(name).exists() {
        return Ok(name.to_owned());
    }
    let stem = candidate
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(name);
    let extension = candidate
        .extension()
        .and_then(|extension| extension.to_str());
    for index in 1..u32::MAX {
        let candidate = extension.map_or_else(
            || format!("{stem} ({index})"),
            |extension| format!("{stem} ({index}).{extension}"),
        );
        if !directory.join(&candidate).exists() {
            return Ok(candidate);
        }
    }
    Err(TransferError::Destination {
        reason: "could not choose an unused inbox file name".to_owned(),
    })
}

#[cfg(test)]
mod inbox_test {
    use super::available_file_name;

    #[test]
    fn available_file_name_uses_the_offered_name_when_free() {
        let directory = tempfile::TempDir::new().unwrap();
        assert_eq!(
            available_file_name(directory.path(), "photo.png").unwrap(),
            "photo.png"
        );
    }
}

/// Root object for one running Mist instance.
///
/// The runtime owns one Iroh endpoint and a registry of independently managed
/// paired devices. `UniFFI` exposes this exact Rust allocation to mobile hosts.
#[derive(Debug)]
pub(crate) struct MistRuntime {
    session: Arc<Session>,
    devices: Arc<dyn DeviceManager>,
    pairing: Arc<dyn Pairing>,
    notifications: Arc<dyn NotificationChannel>,
    clipboard: Arc<dyn Clipboard>,
    media_service: Arc<dyn crate::Media>,
    files: Arc<dyn FileTransfer>,
}

impl MistRuntime {
    pub(crate) fn from_endpoint(
        endpoint: Endpoint,
        device_info: &DeviceInfo,
        store: Arc<dyn StateStore>,
        local_addresses: tokio::sync::watch::Receiver<Option<PublishedAddresses>>,
        blob_directory: std::path::PathBuf,
        files: Arc<dyn BlobStore>,
        file_inbox_directory: Option<std::path::PathBuf>,
    ) -> Result<Arc<Self>, StoreError> {
        let session = Session::new(
            endpoint,
            device_info,
            store,
            local_addresses,
            blob_directory,
            files,
            file_inbox_directory,
        )?;
        session.start();
        let pairing = Arc::new(IrohPairing::new(Arc::clone(&session)));
        let notifications = Arc::new(IrohNotificationChannel::new(Arc::clone(&session)));
        let clipboard = Arc::new(IrohClipboard::new(Arc::clone(&session)));
        let media_service = Arc::new(IrohMedia::new(Arc::clone(&session)));
        let files = Arc::new(crate::transfer::RuntimeFileTransfer::new(Arc::clone(
            &session,
        )));
        let devices = Arc::new(RuntimeDeviceManager::new(Arc::clone(&session)));
        Ok(Arc::new(Self {
            session,
            devices,
            pairing,
            notifications,
            clipboard,
            media_service,
            files,
        }))
    }

    /// Returns the Iroh endpoint owned by this runtime.
    #[must_use]
    #[cfg(test)]
    pub(crate) fn endpoint(&self) -> &Endpoint {
        self.session.endpoint()
    }

    /// Returns any active connection, primarily for low-level diagnostics.
    #[must_use]
    #[cfg(test)]
    pub(crate) fn connection(&self) -> Option<Connection> {
        self.session
            .devices
            .devices()
            .into_iter()
            .find_map(|device| {
                EndpointId::from_str(&device.endpoint_id)
                    .ok()
                    .and_then(|id| self.session.devices.connection(id))
            })
    }
}

impl MistRuntime {
    /// Attaches the platform media backend.
    pub fn attach_media(self: &Arc<Self>, provider: &Arc<dyn MediaProvider>) {
        self.session.attach_media(provider);
    }

    /// Registers the platform command handler used by generated bindings.
    pub fn set_media_command_handler(
        self: &Arc<Self>,
        handler: Arc<dyn crate::MediaCommandHandler>,
    ) {
        self.session.set_media_command_handler(handler);
    }

    async fn shutdown_runtime(&self) {
        let session = Arc::clone(&self.session);
        let executor = session.executor().clone();
        let _ = executor.spawn(async move { session.close().await }).await;
    }
}

#[async_trait]
impl MistApp for MistRuntime {
    fn endpoint_id(&self) -> String {
        self.session.endpoint.id().to_string()
    }

    fn device_manager(&self) -> Arc<dyn DeviceManager> {
        Arc::clone(&self.devices)
    }

    fn pairing(&self) -> Arc<dyn Pairing> {
        Arc::clone(&self.pairing)
    }

    fn notification_channel(&self) -> Arc<dyn NotificationChannel> {
        Arc::clone(&self.notifications)
    }

    fn clipboard(&self) -> Arc<dyn Clipboard> {
        Arc::clone(&self.clipboard)
    }

    fn media(&self) -> Arc<dyn crate::Media> {
        Arc::clone(&self.media_service)
    }

    fn files(&self) -> Arc<dyn FileTransfer> {
        Arc::clone(&self.files)
    }

    async fn shutdown(&self) {
        self.shutdown_runtime().await;
    }
}
