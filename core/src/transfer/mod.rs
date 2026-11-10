//! Blob storage, authorization, and file transfer over Iroh.
//!
//! Payloads use `iroh-blobs` on its own ALPN. Offers travel over the existing
//! authenticated `mist/1` session and authorize one hash for one paired peer.

mod service;

use std::fmt::Debug;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use iroh::EndpointId;
use iroh_blobs::Hash;
use thiserror::Error;
use tokio::sync::{Mutex as AsyncMutex, broadcast};

pub(crate) use service::IrohBlobStore;
pub(crate) use service::RuntimeFileTransfer;

impl From<TransferError> for crate::TransferError {
    fn from(error: TransferError) -> Self {
        match error {
            TransferError::EmptyPath
            | TransferError::NotAFile { .. }
            | TransferError::Empty { .. } => crate::TransferError::Unreadable {
                path: String::new(),
                reason: error.to_string(),
            },
            TransferError::InvalidName
            | TransferError::Storage { .. }
            | TransferError::Transport { .. }
            | TransferError::Destination { .. } => crate::TransferError::Transport {
                reason: error.to_string(),
            },
        }
    }
}

/// Failure while reading, sharing, or receiving a blob.
#[derive(Debug, Error)]
pub(crate) enum TransferError {
    #[error("blob path is empty")]
    EmptyPath,
    #[error("blob path is not a regular file: {path}")]
    NotAFile { path: String },
    #[error("blob is empty: {path}")]
    Empty { path: String },
    #[error("blob name is invalid")]
    InvalidName,
    #[error("blob storage failed: {reason}")]
    Storage { reason: String },
    #[error("blob transfer failed: {reason}")]
    Transport { reason: String },
    #[error("blob destination is not writable: {reason}")]
    Destination { reason: String },
}

/// One imported blob protected for the lifetime of a share.
#[derive(Debug)]
pub(crate) struct StoredBlob {
    pub(crate) hash: Hash,
    pub(crate) _tag: iroh_blobs::api::TempTag,
}

/// Local content-addressed storage used by file transfer.
#[async_trait]
pub(crate) trait BlobStore: Send + Sync + Debug {
    /// Imports one regular file and returns its BLAKE3 hash and protection tag.
    async fn import_file(&self, path: PathBuf) -> Result<StoredBlob, TransferError>;

    /// Downloads one blob from `provider`, resuming any existing partial data.
    async fn download(&self, provider: EndpointId, hash: Hash) -> Result<(), TransferError>;

    /// Writes a completed blob to `destination` and returns its final path.
    async fn export(
        &self,
        hash: Hash,
        destination: PathBuf,
        file_name: &str,
    ) -> Result<PathBuf, TransferError>;

    /// Stops background storage tasks.
    async fn shutdown(&self) -> Result<(), TransferError>;

    /// Serves the blobs protocol on one accepted connection.
    async fn serve(&self, connection: iroh::endpoint::Connection);

    /// Grants one peer access to one offered hash.
    fn authorize(&self, endpoint_id: EndpointId, hash: Hash);

    /// Returns the Iroh blobs ticket for one imported blob.
    fn ticket(&self, hash: Hash) -> iroh_blobs::ticket::BlobTicket;
}

/// Reactive stream of file offers received from paired devices.
#[derive(Debug)]
pub(crate) struct RuntimeFileOfferStream {
    receiver: Arc<AsyncMutex<broadcast::Receiver<crate::FileOffer>>>,
}

/// Reactive stream of files written to the inbox.
#[derive(Debug)]
pub(crate) struct RuntimeFileInboxStream {
    receiver: Arc<AsyncMutex<broadcast::Receiver<crate::ReceivedFile>>>,
}

impl RuntimeFileInboxStream {
    pub(crate) fn new(receiver: broadcast::Receiver<crate::ReceivedFile>) -> Arc<Self> {
        Arc::new(Self {
            receiver: Arc::new(AsyncMutex::new(receiver)),
        })
    }
}

#[async_trait]
impl crate::FileInboxStream for RuntimeFileInboxStream {
    async fn next(&self) -> Result<crate::ReceivedFile, crate::TransferError> {
        self.receiver
            .lock()
            .await
            .recv()
            .await
            .map_err(|error| match error {
                broadcast::error::RecvError::Closed => crate::TransferError::Closed,
                broadcast::error::RecvError::Lagged(count) => {
                    crate::TransferError::Lagged { count }
                }
            })
    }
}

impl RuntimeFileOfferStream {
    pub(crate) fn new(receiver: broadcast::Receiver<crate::FileOffer>) -> Arc<Self> {
        Arc::new(Self {
            receiver: Arc::new(AsyncMutex::new(receiver)),
        })
    }
}

#[async_trait]
impl crate::FileOfferStream for RuntimeFileOfferStream {
    async fn next(&self) -> Result<crate::FileOffer, crate::TransferError> {
        self.receiver
            .lock()
            .await
            .recv()
            .await
            .map_err(|error| match error {
                broadcast::error::RecvError::Closed => crate::TransferError::Closed,
                broadcast::error::RecvError::Lagged(count) => {
                    crate::TransferError::Lagged { count }
                }
            })
    }
}

/// Hashes offered by one peer.
#[derive(Clone, Debug, Default)]
pub(crate) struct ShareAuthorization {
    allowed: Arc<Mutex<std::collections::HashSet<(EndpointId, Hash)>>>,
}

impl ShareAuthorization {
    pub(crate) fn allow(&self, endpoint_id: EndpointId, hash: Hash) {
        self.allowed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert((endpoint_id, hash));
    }

    pub(crate) fn allows(&self, endpoint_id: EndpointId, hash: Hash) -> bool {
        self.allowed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&(endpoint_id, hash))
    }
}

#[cfg(test)]
mod test;
