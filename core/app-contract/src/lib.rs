//! Stable application-facing API for Mist.
//!
//! This crate deliberately contains only platform-neutral values, errors, and
//! behavioral traits. Networking, persistence, executors, and wire types are
//! implementation details of `mist-core`.

mod app;
mod clipboard;
mod device;
mod media;
mod notification;
mod pairing;
mod transfer;

pub use app::MistApp;
pub use clipboard::{
    Clipboard, ClipboardContent, ClipboardError, ClipboardEventStream, ReceivedClipboard,
};
pub use device::{
    ConnectionTransport, DeviceClass, DeviceError, DeviceEvent, DeviceEventError, DeviceEventKind,
    DeviceEventStream, DeviceInfo, DeviceManager, DeviceStatus, ManagedDevice, Platform,
};
pub use media::{
    Media, MediaArtwork, MediaCapabilities, MediaCommand, MediaCommandHandler, MediaError,
    MediaEvent, MediaEventKind, MediaEventStream, MediaMetadata, MediaOrigin, MediaPlayer,
    PlaybackStatus,
};
pub use notification::{
    ActivatedNotificationAction, Notification, NotificationAction, NotificationActivationStream,
    NotificationChannel, NotificationError, NotificationEventStream, NotificationImage,
    NotificationKind, ReceivedNotification,
};
pub use pairing::{PairedPeer, Pairing, PairingError, PairingOffer};
pub use transfer::{
    FileInboxStream, FileOffer, FileOfferStream, FileShare, FileTransfer, ReceivedFile,
    TransferError,
};

uniffi::setup_scaffolding!();
