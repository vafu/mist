//! Shared native runtime for Mist.
//!
//! The app-facing API is defined by `mist-app-contract`. This crate owns its
//! Iroh, persistence, protocol, and executor implementations and exposes only a
//! narrow host construction seam.

mod clipboard;
mod device;
mod media;
mod notification;
mod pairing;
mod protocol;
mod runtime;
mod storage;
mod transfer;

pub mod host;
#[cfg(feature = "probe")]
pub mod probe;

pub(crate) use mist_app_contract::{
    ActivatedNotificationAction, Clipboard, ClipboardContent, ClipboardError, ConnectionTransport,
    DeviceClass, DeviceError, DeviceEvent, DeviceEventKind, DeviceInfo, DeviceManager,
    DeviceStatus, ManagedDevice, Media, MediaCapabilities, MediaCommand, MediaCommandHandler,
    MediaError, MediaEvent, MediaMetadata, MediaOrigin, MediaPlayer, MistApp, Notification,
    NotificationAction, NotificationChannel, NotificationError, NotificationImage,
    NotificationKind, Pairing, Platform, PlaybackStatus, ReceivedClipboard, ReceivedNotification,
};

/// Iroh application protocol negotiated by every Mist connection.
pub(crate) const MIST_ALPN: &[u8] = b"mist/1";

uniffi::setup_scaffolding!();
