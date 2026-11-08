use std::fmt::Debug;
use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;

use crate::DeviceInfo;

/// What a platform notification represents.
///
/// The originating platform classifies the notification; the shared runtime
/// decides which feature consumes it. Media transport updates are dropped by
/// the generic notification feature and belong to the media feature instead.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum NotificationKind {
    /// Interactive content such as messages, alerts, and reminders.
    Message,
    /// Media playback transport state, including track and playback controls.
    Media,
    /// Any other notification the platform could not classify.
    Other,
}

/// An image attached to a notification.
///
/// `path` is an absolute path to a regular file the caller owns. Runtimes copy
/// or stream it; the path never identifies a peer-address or network resource.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct NotificationImage {
    pub mime_type: String,
    pub path: String,
}

/// Platform-neutral notification exchanged between Mist devices.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct Notification {
    pub id: String,
    pub application: String,
    pub title: String,
    pub body: String,
    pub timestamp_ms: i64,
    pub silent: bool,
    pub kind: NotificationKind,
    /// Small application icon, typically 48-96 px.
    pub app_icon: Option<NotificationImage>,
    /// Larger content image such as a photo or album art.
    pub image: Option<NotificationImage>,
}

/// Notification received from an authenticated peer.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct ReceivedNotification {
    pub source_endpoint_id: String,
    pub source_device: DeviceInfo,
    pub notification: Notification,
}

/// Failure while sending or receiving notifications.
#[derive(Debug, Error, uniffi::Error)]
pub enum NotificationError {
    #[error("invalid endpoint ID: {reason}")]
    InvalidEndpointId { reason: String },
    #[error("unknown paired device {endpoint_id}")]
    UnknownDevice { endpoint_id: String },
    #[error("no device is selected")]
    NoSelectedDevices,
    #[error("device {endpoint_id} is disconnected")]
    Disconnected { endpoint_id: String },
    #[error("notification transport failed: {reason}")]
    Transport { reason: String },
    #[error("the Mist notification stream is closed")]
    Closed,
    #[error("notification subscriber missed {count} events")]
    Lagged { count: u64 },
    #[error("failed to run notification work on the Mist native executor: {reason}")]
    Executor { reason: String },
}

/// Event-driven stream of remote notifications.
#[uniffi::export]
#[async_trait]
pub trait NotificationEventStream: Send + Sync + Debug {
    async fn next(&self) -> Result<ReceivedNotification, NotificationError>;
}

/// Sends notifications and creates reactive receive streams.
#[uniffi::export]
#[async_trait]
pub trait NotificationChannel: Send + Sync + Debug {
    async fn push(
        &self,
        notification: Notification,
        targets: Vec<String>,
    ) -> Result<(), NotificationError>;

    fn subscribe(&self) -> Arc<dyn NotificationEventStream>;
}
