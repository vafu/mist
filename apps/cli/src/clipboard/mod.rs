//! Platform clipboard contract for the Linux host.
//!
//! The core runtime owns echo suppression and message scheduling; this module
//! only observes and applies native clipboard content.

mod wayland;

use std::fmt::Debug;

use mist_app_contract::ClipboardContent;

pub use wayland::WaylandClipboard;

/// Failure while observing or applying native clipboard content.
#[derive(Debug, thiserror::Error)]
pub enum LocalClipboardError {
    /// The platform clipboard is unavailable in this session.
    #[error("no usable clipboard backend: {reason}")]
    Unavailable { reason: String },
    /// Reading clipboard content failed.
    #[error("failed to read clipboard content: {reason}")]
    Read { reason: String },
    /// Writing clipboard content failed.
    #[error("failed to write clipboard content: {reason}")]
    Write { reason: String },
    /// Clipboard content exceeded the accepted size.
    #[error("clipboard content exceeds the size limit")]
    TooLarge,
    /// Clipboard content was not valid UTF-8 where text was expected.
    #[error("clipboard content is not valid UTF-8")]
    InvalidText,
}

/// Observes local clipboard changes and applies remote content locally.
pub trait LocalClipboardProvider: Send + Sync + Debug {
    /// Starts watching the regular clipboard.
    ///
    /// # Errors
    ///
    /// Returns [`LocalClipboardError::Unavailable`] when no backend is usable.
    fn watch(&self) -> Result<Box<dyn LocalClipboardWatcher>, LocalClipboardError>;

    /// Replaces the local clipboard with `content`.
    ///
    /// # Errors
    ///
    /// Returns an error when the content cannot be written to the clipboard.
    fn apply(&self, content: &ClipboardContent) -> Result<(), LocalClipboardError>;
}

/// Blocking source of local clipboard changes.
pub trait LocalClipboardWatcher: Send {
    /// Blocks until new local content is available.
    ///
    /// Returns `Ok(None)` when the watcher was cancelled or the clipboard was
    /// cleared.
    ///
    /// # Errors
    ///
    /// Returns an error when the Wayland connection or data transfer fails.
    fn next(&mut self) -> Result<Option<ClipboardContent>, LocalClipboardError>;
}
