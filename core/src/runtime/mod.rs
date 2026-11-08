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
use crate::protocol::wire::{ControlFrame, PairingAuthenticate, control_frame};
use crate::protocol::{
    ClipboardMetadata, address_snapshot, decode_address_snapshot, decode_clipboard_item,
    decode_media_command, decode_media_snapshot, decode_notification_with_images,
    decode_session_hello, encode_media_command, encode_media_snapshot, read_control_frame,
    session_hello, write_control_frame,
};
use crate::storage::{AddressSource, StateStore, StoreError};
use crate::transfer::{BlobMetadata, inspect_file, receive_blob};
use crate::{
    Clipboard, ClipboardContent, ClipboardError, DeviceError, DeviceInfo, DeviceManager, MIST_ALPN,
    ManagedDevice, MediaCommand, MediaError, MediaEvent, MediaPlayer, MistApp, Notification,
    NotificationChannel, NotificationError, NotificationKind, Pairing, ReceivedClipboard,
    ReceivedNotification,
};

pub(crate) use builder::MistBuilder;

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
    blob_directory: std::path::PathBuf,
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
    ) -> Result<Arc<Self>, StoreError> {
        let (notifications, _) = broadcast::channel(256);
        let (clipboard, _) = broadcast::channel(64);
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
            blob_directory,
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
        let snapshot = encode_media_snapshot(&self.media.list());
        self.executor.spawn(async move {
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
        let snapshot = encode_media_snapshot(&players);
        let connections = self.devices.selected_connections();
        for (endpoint_id, connection) in connections {
            let Some(connection) = connection else {
                continue;
            };
            let _ = endpoint_id;
            let snapshot = snapshot.clone();
            self.executor.spawn(async move {
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

    async fn receive_feature_frame(
        &self,
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
                let notification = self.receive_notification_images(&mut stream, decoded).await;
                let Some(source_device) = self.devices.device_info(endpoint_id) else {
                    return;
                };
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
                let (content, sha256) = match metadata {
                    ClipboardMetadata::Text { text } => {
                        let fingerprint = crate::runtime::outbox::text_fingerprint(&text);
                        (ClipboardContent::Text { text }, fingerprint)
                    }
                    ClipboardMetadata::Blob {
                        mime_type,
                        size,
                        sha256,
                    } => {
                        let metadata = BlobMetadata {
                            mime_type,
                            size,
                            sha256,
                        };
                        let Ok(path) =
                            receive_blob(&mut stream, &self.blob_directory, &metadata).await
                        else {
                            return;
                        };
                        (
                            ClipboardContent::Blob {
                                mime_type: metadata.mime_type,
                                path: path.to_string_lossy().into_owned(),
                            },
                            sha256,
                        )
                    }
                };
                self.outbox.remember_received(sha256);
                let _ = self.clipboard.send(ReceivedClipboard {
                    source_endpoint_id: endpoint_id.to_string(),
                    source_device,
                    content,
                    sha256: hex(&sha256),
                });
            }
            Some(control_frame::Body::MediaSnapshot(snapshot)) => {
                self.receive_media_snapshot(endpoint_id, snapshot);
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
        stream: &mut RecvStream,
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
            if let Ok(path) = receive_blob(stream, &self.blob_directory, &metadata).await
                && let Some(image) = slot.as_mut()
            {
                image.path = path.to_string_lossy().into_owned();
            }
        }
        notification
    }

    fn receive_media_snapshot(
        &self,
        endpoint_id: EndpointId,
        snapshot: crate::protocol::wire::MediaSnapshot,
    ) {
        let Ok(players) = decode_media_snapshot(snapshot) else {
            return;
        };
        let Some(device) = self.devices.device_info(endpoint_id) else {
            return;
        };
        let players = players
            .iter()
            .map(|player| crate::media::remote_player(&endpoint_id.to_string(), &device, player))
            .collect::<Vec<_>>();
        self.media
            .set_remote(&endpoint_id.to_string(), &device, &players);
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
                session.route_connection(connection).await;
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

    pub(crate) fn push_notification(
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

        crate::protocol::validate_notification_images(notification).map_err(|error| {
            NotificationError::Transport {
                reason: error.to_string(),
            }
        })?;

        for endpoint_id in targets {
            self.schedule(endpoint_id, Outbound::Notification(notification.clone()));
        }
        Ok(())
    }

    pub(crate) fn push_clipboard(
        self: &Arc<Self>,
        content: &ClipboardContent,
        targets: Vec<String>,
    ) -> Result<(), ClipboardError> {
        let metadata = clipboard_blob_metadata(content)?;

        let targets = self.resolve_targets(targets).map_err(|error| match error {
            TargetError::Invalid(reason) => ClipboardError::InvalidEndpointId { reason },
            TargetError::Unknown(endpoint_id) => ClipboardError::UnknownDevice { endpoint_id },
            TargetError::NoSelected => ClipboardError::NoSelectedDevices,
        })?;

        for endpoint_id in targets {
            self.schedule(
                endpoint_id,
                Outbound::Clipboard {
                    content: content.clone(),
                    metadata: metadata.clone(),
                },
            );
        }
        Ok(())
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

fn clipboard_blob_metadata(
    content: &ClipboardContent,
) -> Result<Option<BlobMetadata>, ClipboardError> {
    match content {
        ClipboardContent::Text { text } => {
            if text.is_empty() {
                Err(ClipboardError::Empty)
            } else {
                Ok(None)
            }
        }
        ClipboardContent::Blob { mime_type, path } => {
            inspect_file(mime_type, std::path::Path::new(path))
                .map(Some)
                .map_err(|error| ClipboardError::Unreadable {
                    reason: error.to_string(),
                })
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
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
}

impl MistRuntime {
    pub(crate) fn from_endpoint(
        endpoint: Endpoint,
        device_info: &DeviceInfo,
        store: Arc<dyn StateStore>,
        local_addresses: tokio::sync::watch::Receiver<Option<PublishedAddresses>>,
        blob_directory: std::path::PathBuf,
    ) -> Result<Arc<Self>, StoreError> {
        let session = Session::new(
            endpoint,
            device_info,
            store,
            local_addresses,
            blob_directory,
        )?;
        session.start();
        let pairing = Arc::new(IrohPairing::new(Arc::clone(&session)));
        let notifications = Arc::new(IrohNotificationChannel::new(Arc::clone(&session)));
        let clipboard = Arc::new(IrohClipboard::new(Arc::clone(&session)));
        let media_service = Arc::new(IrohMedia::new(Arc::clone(&session)));
        let devices = Arc::new(RuntimeDeviceManager::new(Arc::clone(&session)));
        Ok(Arc::new(Self {
            session,
            devices,
            pairing,
            notifications,
            clipboard,
            media_service,
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

    async fn shutdown(&self) {
        self.shutdown_runtime().await;
    }
}
