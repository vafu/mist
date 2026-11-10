use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointId};
use iroh_blobs::api::TempTag;
use iroh_blobs::api::blobs::AddPathOptions;
use iroh_blobs::api::blobs::ImportMode;
use iroh_blobs::api::downloader::Downloader;
use iroh_blobs::provider::events::{
    AbortReason, ConnectMode, EventMask, EventSender, ObserveMode, ProviderMessage, RequestMode,
};
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::{BlobFormat, Hash};

use super::{BlobStore, ShareAuthorization, StoredBlob, TransferError};
use crate::runtime::Session;

/// Production blob storage backed by an on-disk `iroh-blobs` store.
#[derive(Debug)]
pub(crate) struct IrohBlobStore {
    store: FsStore,
    downloader: Downloader,
    authorization: ShareAuthorization,
    endpoint_addr: iroh::EndpointAddr,
}

impl IrohBlobStore {
    pub(crate) async fn load(
        root: impl AsRef<Path>,
        endpoint: &Endpoint,
        authorization: ShareAuthorization,
    ) -> Result<Self, TransferError> {
        let store = FsStore::load(root)
            .await
            .map_err(|error| TransferError::Storage {
                reason: error.to_string(),
            })?;
        let downloader = Downloader::builder(store.as_ref(), endpoint).build();
        Ok(Self {
            store,
            downloader,
            authorization,
            endpoint_addr: endpoint.addr(),
        })
    }

    pub(crate) fn authorize(&self, endpoint_id: EndpointId, hash: Hash) {
        self.authorization.allow(endpoint_id, hash);
    }

    fn ticket_for(&self, hash: Hash) -> iroh_blobs::ticket::BlobTicket {
        iroh_blobs::ticket::BlobTicket::new(
            self.endpoint_addr.clone(),
            hash,
            iroh_blobs::BlobFormat::Raw,
        )
    }
}

#[async_trait]
impl BlobStore for IrohBlobStore {
    async fn import_file(&self, path: PathBuf) -> Result<StoredBlob, TransferError> {
        validate_file(&path)?;
        let absolute = std::path::absolute(&path).map_err(|error| TransferError::Storage {
            reason: error.to_string(),
        })?;
        let tag: TempTag = self
            .store
            .add_path_with_opts(AddPathOptions {
                path: absolute,
                format: BlobFormat::Raw,
                mode: ImportMode::Copy,
            })
            .temp_tag()
            .await
            .map_err(|error| TransferError::Storage {
                reason: error.to_string(),
            })?;
        Ok(StoredBlob {
            hash: tag.hash(),
            _tag: tag,
        })
    }

    async fn download(&self, provider: EndpointId, hash: Hash) -> Result<(), TransferError> {
        self.downloader
            .download(hash, [provider])
            .await
            .map_err(|error| TransferError::Transport {
                reason: error.to_string(),
            })
    }

    async fn export(
        &self,
        hash: Hash,
        destination: PathBuf,
        file_name: &str,
    ) -> Result<PathBuf, TransferError> {
        let file_name = safe_file_name(file_name)?;
        tokio::fs::create_dir_all(&destination)
            .await
            .map_err(|error| TransferError::Destination {
                reason: error.to_string(),
            })?;
        let target = destination.join(file_name);
        self.store
            .export(hash, &target)
            .await
            .map_err(|error| TransferError::Destination {
                reason: error.to_string(),
            })?;
        Ok(target)
    }

    async fn shutdown(&self) -> Result<(), TransferError> {
        self.store
            .shutdown()
            .await
            .map_err(|error| TransferError::Storage {
                reason: error.to_string(),
            })
    }

    async fn serve(&self, connection: Connection) {
        let events = authorized_events(self.authorization.clone());
        iroh_blobs::provider::handle_connection(connection, self.store.clone().into(), events)
            .await;
    }

    fn authorize(&self, endpoint_id: EndpointId, hash: Hash) {
        IrohBlobStore::authorize(self, endpoint_id, hash);
    }

    fn ticket(&self, hash: Hash) -> iroh_blobs::ticket::BlobTicket {
        self.ticket_for(hash)
    }
}

fn authorized_events(authorization: ShareAuthorization) -> EventSender {
    let mask = EventMask {
        connected: ConnectMode::Notify,
        get: RequestMode::Intercept,
        get_many: RequestMode::Disabled,
        observe: ObserveMode::Intercept,
        ..EventMask::DEFAULT
    };
    let (events, mut receiver) = EventSender::channel(64, mask);
    let peers: Arc<Mutex<HashMap<u64, EndpointId>>> = Arc::new(Mutex::new(HashMap::new()));
    n0_future::task::spawn(async move {
        while let Some(message) = receiver.recv().await {
            match message {
                ProviderMessage::ClientConnectedNotify(message) => {
                    let Some(endpoint_id) = message.endpoint_id else {
                        continue;
                    };
                    peers
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .insert(message.connection_id, endpoint_id);
                }
                ProviderMessage::ConnectionClosed(message) => {
                    peers
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .remove(&message.connection_id);
                }
                ProviderMessage::GetRequestReceived(message) => {
                    let endpoint_id = peers
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .get(&message.connection_id)
                        .copied();
                    let allowed = message.request.ranges.is_blob()
                        && endpoint_id.is_some_and(|endpoint_id| {
                            authorization.allows(endpoint_id, message.request.hash)
                        });
                    let _ = message
                        .tx
                        .send(if allowed {
                            Ok(())
                        } else {
                            Err(AbortReason::Permission)
                        })
                        .await;
                }
                ProviderMessage::ObserveRequestReceived(message) => {
                    let endpoint_id = peers
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .get(&message.connection_id)
                        .copied();
                    let allowed = endpoint_id.is_some_and(|endpoint_id| {
                        authorization.allows(endpoint_id, message.request.hash)
                    });
                    let _ = message
                        .tx
                        .send(if allowed {
                            Ok(())
                        } else {
                            Err(AbortReason::Permission)
                        })
                        .await;
                }
                _ => {}
            }
        }
    });
    events
}

fn validate_file(path: &Path) -> Result<(), TransferError> {
    if path.as_os_str().is_empty() {
        return Err(TransferError::EmptyPath);
    }
    let metadata = std::fs::metadata(path).map_err(|error| TransferError::NotAFile {
        path: format!("{}: {error}", path.display()),
    })?;
    if !metadata.is_file() {
        return Err(TransferError::NotAFile {
            path: path.display().to_string(),
        });
    }
    if metadata.len() == 0 {
        return Err(TransferError::Empty {
            path: path.display().to_string(),
        });
    }
    Ok(())
}

fn safe_file_name(name: &str) -> Result<String, TransferError> {
    let candidate = Path::new(name);
    if candidate.file_name().and_then(|name| name.to_str()) != Some(name)
        || name == "."
        || name == ".."
    {
        return Err(TransferError::InvalidName);
    }
    Ok(name.to_owned())
}

/// File transfer service exposed by the runtime.
#[derive(Clone, Debug)]
pub(crate) struct RuntimeFileTransfer {
    session: Arc<Session>,
}

impl RuntimeFileTransfer {
    pub(crate) fn new(session: Arc<Session>) -> Self {
        Self { session }
    }
}

#[async_trait]
impl crate::FileTransfer for RuntimeFileTransfer {
    async fn share(
        &self,
        path: String,
        targets: Vec<String>,
    ) -> Result<crate::FileShare, crate::TransferError> {
        self.session.share_file(path, targets).await
    }

    fn subscribe(&self) -> Arc<dyn crate::FileOfferStream> {
        crate::transfer::RuntimeFileOfferStream::new(self.session.subscribe_files())
    }

    fn subscribe_inbox(&self) -> Arc<dyn crate::FileInboxStream> {
        crate::transfer::RuntimeFileInboxStream::new(self.session.subscribe_inbox())
    }

    async fn accept(&self, offer: crate::FileOffer) -> Result<String, crate::TransferError> {
        self.session.accept_file(offer).await
    }
}
