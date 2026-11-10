//! Bidirectional media player control over the authenticated Mist session.
//!
//! The runtime owns player state and command routing. Platform hosts only
//! discover local players and apply transport commands.

mod registry;
mod service;

use std::fmt::Debug;
use std::sync::Arc;

use async_trait::async_trait;
use mist_app_contract::{
    MediaCommand, MediaError, MediaEvent, MediaEventStream, MediaMetadata, MediaOrigin,
    MediaPlayer, PlaybackStatus,
};

pub(crate) use registry::{MediaRegistry, remote_player};
pub(crate) use service::IrohMedia;

/// Failure while talking to the platform media stack.
#[derive(Debug, thiserror::Error)]
pub enum MediaProviderError {
    #[error("media backend unavailable: {reason}")]
    Unavailable { reason: String },
    #[error("media backend rejected the command: {reason}")]
    Command { reason: String },
}

/// One player reported by a platform media backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalPlayer {
    pub id: String,
    pub name: String,
    pub status: PlaybackStatus,
    pub metadata: MediaMetadata,
    /// Transport commands the player accepts.
    pub commands: Vec<MediaCommand>,
    pub position_ms: Option<u64>,
}

impl LocalPlayer {
    /// Whether the player accepts `command`.
    #[must_use]
    pub fn supports(&self, command: MediaCommand) -> bool {
        self.commands.contains(&command)
    }
}

/// Platform media integration seam.
///
/// Implementations observe local players and apply transport commands. They do
/// not know about peers, routing, or the wire protocol.
#[async_trait::async_trait]
pub trait MediaProvider: Send + Sync + Debug {
    /// Starts a blocking watcher for local player changes.
    ///
    /// # Errors
    ///
    /// Returns an error when the platform backend is unavailable.
    fn watch(&self) -> Result<Box<dyn MediaWatcher>, MediaProviderError>;

    /// Applies a transport command to a local player.
    ///
    /// # Errors
    ///
    /// Returns an error when the player is unknown or rejects the command.
    async fn command(
        &self,
        player_id: &str,
        command: MediaCommand,
    ) -> Result<(), MediaProviderError>;
}

/// Blocking source of local player snapshots.
pub trait MediaWatcher: Send {
    /// Blocks until the local player set changes.
    ///
    /// Returns `Ok(None)` when the watcher stopped and no further updates are
    /// expected.
    ///
    /// # Errors
    ///
    /// Returns an error when the platform backend fails.
    fn next(&mut self) -> Result<Option<Vec<LocalPlayer>>, MediaProviderError>;
}

#[derive(Debug)]
pub(crate) struct RuntimeMediaEventStream {
    receiver: Arc<tokio::sync::Mutex<tokio::sync::broadcast::Receiver<MediaEvent>>>,
}

impl RuntimeMediaEventStream {
    pub(crate) fn new(receiver: tokio::sync::broadcast::Receiver<MediaEvent>) -> Arc<Self> {
        Arc::new(Self {
            receiver: Arc::new(tokio::sync::Mutex::new(receiver)),
        })
    }
}

#[async_trait]
impl MediaEventStream for RuntimeMediaEventStream {
    async fn next(&self) -> Result<MediaEvent, MediaError> {
        self.receiver
            .lock()
            .await
            .recv()
            .await
            .map_err(|error| match error {
                tokio::sync::broadcast::error::RecvError::Closed => MediaError::Closed,
                tokio::sync::broadcast::error::RecvError::Lagged(count) => {
                    MediaError::Lagged { count }
                }
            })
    }
}

/// Builds the app-visible snapshot for one local player.
pub(crate) fn local_player(device: &crate::DeviceInfo, player: &LocalPlayer) -> MediaPlayer {
    MediaPlayer {
        id: player.id.clone(),
        endpoint_id: String::new(),
        name: player.name.clone(),
        device: device.clone(),
        origin: MediaOrigin::Local,
        status: player.status,
        metadata: player.metadata.clone(),
        artwork_path: None,
        capabilities: mist_app_contract::MediaCapabilities {
            commands: player.commands.clone(),
        },
        position_ms: player.position_ms,
    }
}
