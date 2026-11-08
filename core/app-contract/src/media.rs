use std::fmt::Debug;
use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;

use crate::DeviceInfo;

/// Which device owns a media player.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum MediaOrigin {
    /// A player running on this device.
    Local,
    /// A player running on a paired device.
    Remote,
}

/// Playback status reported by a media player.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum PlaybackStatus {
    Stopped,
    Playing,
    Paused,
}

/// Transport operations a player may accept.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum MediaCommand {
    Play,
    Pause,
    PlayPause,
    Stop,
    Next,
    Previous,
}

/// Metadata describing the currently loaded media.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct MediaMetadata {
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Remote artwork reference; empty when none is available.
    pub art_url: String,
    /// Track length in milliseconds; `None` when unknown.
    pub duration_ms: Option<u64>,
}

/// Transport operations supported by a player.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct MediaCapabilities {
    /// Commands this player accepts.
    pub commands: Vec<MediaCommand>,
}

impl MediaCapabilities {
    /// Whether the player accepts `command`.
    #[must_use]
    pub fn supports(&self, command: MediaCommand) -> bool {
        self.commands.contains(&command)
    }
}

/// One controllable media player.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct MediaPlayer {
    /// Stable player identifier, unique within its origin.
    pub id: String,
    /// Owning endpoint for remote players; empty for local players.
    pub endpoint_id: String,
    /// Human-readable player name.
    pub name: String,
    /// Device hosting the player.
    pub device: DeviceInfo,
    pub origin: MediaOrigin,
    pub status: PlaybackStatus,
    pub metadata: MediaMetadata,
    pub capabilities: MediaCapabilities,
    /// Position in milliseconds; `None` when unknown.
    pub position_ms: Option<u64>,
}

/// Kind of change published by the media service.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum MediaEventKind {
    Added,
    Changed,
    Removed,
}

/// Reactive media update.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct MediaEvent {
    pub kind: MediaEventKind,
    pub player: MediaPlayer,
}

/// Failure while managing or controlling media players.
#[derive(Debug, Error, uniffi::Error)]
pub enum MediaError {
    #[error("unknown media player {player_id}")]
    UnknownPlayer { player_id: String },
    #[error("media player {player_id} does not support that command")]
    UnsupportedCommand { player_id: String },
    #[error("media transport failed: {reason}")]
    Transport { reason: String },
    #[error("the media event stream is closed")]
    Closed,
    #[error("media subscriber missed {count} events")]
    Lagged { count: u64 },
}

/// Event-driven stream of media player changes.
#[uniffi::export]
#[async_trait]
pub trait MediaEventStream: Send + Sync + Debug {
    /// Suspends until the next media change is available.
    async fn next(&self) -> Result<MediaEvent, MediaError>;
}

/// Media players visible to this runtime, local and remote.
#[uniffi::export]
#[async_trait]
pub trait Media: Send + Sync + Debug {
    /// Returns current snapshots of every known player.
    fn list(&self) -> Vec<MediaPlayer>;

    /// Creates an independent reactive event stream.
    fn subscribe(&self) -> Arc<dyn MediaEventStream>;

    /// Sends a transport command to a remote player.
    async fn command(&self, player_id: String, command: MediaCommand) -> Result<(), MediaError>;
}
