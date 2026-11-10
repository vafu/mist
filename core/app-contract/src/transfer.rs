use std::fmt::Debug;
use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;

/// A file made available to trusted peers.
///
/// The file is hashed and imported before the offer is published. Paths never
/// cross this contract: peers receive the ticket and metadata instead.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct FileShare {
    /// Stable offer identifier scoped to the sending runtime.
    pub id: String,
    /// Suggested file name shown to the receiver.
    pub name: String,
    /// File size in bytes.
    pub size: u64,
    /// Iroh blobs ticket suitable for direct download.
    pub ticket: String,
}

/// A file offered by another authenticated device.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct FileOffer {
    /// Device that made the offer.
    pub source_endpoint_id: String,
    /// Device metadata for the sender.
    pub source_device: crate::DeviceInfo,
    /// Opaque offer identifier used to accept it.
    pub id: String,
    /// Suggested file name.
    pub name: String,
    /// File size in bytes.
    pub size: u64,
    /// Iroh blobs ticket for the offered file.
    pub ticket: String,
}

/// A file accepted and written into the inbox.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct ReceivedFile {
    /// Device that offered the file.
    pub source_endpoint_id: String,
    /// Device metadata for the sender.
    pub source_device: crate::DeviceInfo,
    /// Suggested file name from the offer.
    pub name: String,
    /// Absolute path of the completed inbox file.
    pub path: String,
    /// File size in bytes.
    pub size: u64,
}

/// Failure while sharing or receiving a file.
#[derive(Debug, Error, uniffi::Error)]
pub enum TransferError {
    #[error("invalid endpoint ID: {reason}")]
    InvalidEndpointId { reason: String },
    #[error("unknown paired device {endpoint_id}")]
    UnknownDevice { endpoint_id: String },
    #[error("no file was shared at {path}")]
    Unreadable { path: String, reason: String },
    #[error("no device is selected")]
    NoSelectedDevices,
    #[error("file or directory is empty: {path}")]
    Empty { path: String },
    #[error("the destination directory is not writable: {reason}")]
    Destination { reason: String },
    #[error("no inbox directory is configured for incoming files")]
    InboxNotConfigured,
    #[error("the file offer {id} is no longer available")]
    UnknownOffer { id: String },
    #[error("the file is only available from an untrusted device")]
    UntrustedSource,
    #[error("file transfer failed: {reason}")]
    Transport { reason: String },
    #[error("the Mist file offer stream is closed")]
    Closed,
    #[error("file subscriber missed {count} events")]
    Lagged { count: u64 },
}

/// Reactive stream of file offers from peers.
#[uniffi::export]
#[async_trait]
pub trait FileOfferStream: Send + Sync + Debug {
    /// Suspends until another peer offers a file.
    async fn next(&self) -> Result<FileOffer, TransferError>;
}

/// Reactive stream of files accepted into the inbox.
#[uniffi::export]
#[async_trait]
pub trait FileInboxStream: Send + Sync + Debug {
    /// Suspends until another accepted file is available.
    async fn next(&self) -> Result<ReceivedFile, TransferError>;
}

/// Shares files with trusted peers and downloads their offers.
#[uniffi::export]
#[async_trait]
pub trait FileTransfer: Send + Sync + Debug {
    /// Imports `path` and offers it to paired devices.
    ///
    /// Passing explicit `targets` publishes only to those devices. An empty
    /// list offers the file to every currently selected device.
    async fn share(&self, path: String, targets: Vec<String>) -> Result<FileShare, TransferError>;

    /// Creates an independent stream of incoming offers.
    fn subscribe(&self) -> Arc<dyn FileOfferStream>;

    /// Downloads an accepted offer into the configured inbox directory.
    ///
    /// Returns the absolute path of the completed file. The download is
    /// resumed when the same offer is requested again after interruption.
    async fn accept(&self, offer: FileOffer) -> Result<String, TransferError>;

    /// Creates an independent stream of files accepted into the inbox.
    fn subscribe_inbox(&self) -> Arc<dyn FileInboxStream>;
}
