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
    /// Local artwork file published to peers; `None` when unavailable.
    pub artwork: Option<MediaArtwork>,
    /// Track length in milliseconds; `None` when unknown.
    pub duration_ms: Option<u64>,
}

/// Artwork attached to the currently loaded media.
///
/// `path` is an absolute path to a regular image file the caller owns. The
/// runtime streams it to peers; the path never crosses the network.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct MediaArtwork {
    pub mime_type: String,
    pub path: String,
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
    /// Local artwork file received from a peer; `None` for local players.
    pub artwork_path: Option<String>,
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

/// Platform callback for transport commands addressed to local players.
#[uniffi::export(callback_interface)]
pub trait MediaCommandHandler: Send + Sync + Debug {
    /// Applies `command` to the local player identified by `player_id`.
    ///
    /// # Errors
    ///
    /// Returns an error when the player is unknown or rejects the command.
    fn command(&self, player_id: String, command: MediaCommand) -> Result<(), MediaError>;
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
    /// Sends a transport command to a remote player.
    ///
    /// # Errors
    ///
    /// Returns an error when the player is unknown or the transport fails.
    async fn command(&self, player_id: String, command: MediaCommand) -> Result<(), MediaError>;

    /// Publishes this device's local player snapshot.
    ///
    /// Apps call this whenever platform playback state changes. It is the only
    /// way local players enter the shared runtime; peers receive the snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when the snapshot cannot be recorded.
    fn publish_local(&self, players: Vec<MediaPlayer>) -> Result<(), MediaError>;
}
