use std::fmt::Debug;
use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;

use crate::DeviceInfo;

/// Clipboard content shared between Mist devices.
///
/// Binary content is referenced by an app-private file path so large payloads
/// never cross the binding or Binder boundary.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum ClipboardContent {
    /// UTF-8 plain text.
    Text { text: String },
    /// A binary payload stored in an app-private file.
    Blob { mime_type: String, path: String },
}

/// Clipboard content received from an authenticated peer.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct ReceivedClipboard {
    pub source_endpoint_id: String,
    pub source_device: DeviceInfo,
    pub content: ClipboardContent,
    /// BLAKE3 content identifier. The field name is retained for binding compatibility.
    pub sha256: String,
}

/// Failure while sending or receiving clipboard content.
#[derive(Debug, Error, uniffi::Error)]
pub enum ClipboardError {
    #[error("invalid endpoint ID: {reason}")]
    InvalidEndpointId { reason: String },
    #[error("unknown paired device {endpoint_id}")]
    UnknownDevice { endpoint_id: String },
    #[error("no device is selected")]
    NoSelectedDevices,
    #[error("device {endpoint_id} is disconnected")]
    Disconnected { endpoint_id: String },
    #[error("clipboard content is too large")]
    TooLarge,
    #[error("clipboard content is empty")]
    Empty,
    #[error("unsupported clipboard MIME type: {mime_type}")]
    UnsupportedContent { mime_type: String },
    #[error("clipboard blob is not readable: {reason}")]
    Unreadable { reason: String },
    #[error("clipboard transport failed: {reason}")]
    Transport { reason: String },
    #[error("the Mist clipboard stream is closed")]
    Closed,
    #[error("clipboard subscriber missed {count} events")]
    Lagged { count: u64 },
    #[error("failed to run clipboard work on the Mist native executor: {reason}")]
    Executor { reason: String },
}

/// Event-driven stream of clipboard content from peers.
#[uniffi::export]
#[async_trait]
pub trait ClipboardEventStream: Send + Sync + Debug {
    /// Suspends until the next clipboard item is available.
    async fn next(&self) -> Result<ReceivedClipboard, ClipboardError>;
}

/// Sends clipboard content to paired devices.
#[uniffi::export]
#[async_trait]
pub trait Clipboard: Send + Sync + Debug {
    /// Sends content to explicit targets, or to all selected devices when
    /// `targets` is empty.
    async fn push(
        &self,
        content: ClipboardContent,
        targets: Vec<String>,
    ) -> Result<(), ClipboardError>;

    /// Creates an independent reactive receive stream.
    fn subscribe(&self) -> Arc<dyn ClipboardEventStream>;
}
