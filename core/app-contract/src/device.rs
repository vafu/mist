use std::fmt::Debug;
use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;

/// The physical form of a Mist device.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum DeviceClass {
    Phone,
    Tablet,
    Laptop,
    Desktop,
    Server,
    Other,
}

/// The operating-system family hosting Mist.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum Platform {
    Android,
    Linux,
    Ios,
    Macos,
    Windows,
    Other,
}

/// User-visible information identifying a device.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct DeviceInfo {
    pub display_name: String,
    pub device_class: DeviceClass,
    pub platform: Platform,
}

/// Current connection state for a paired device.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum DeviceStatus {
    Disconnected,
    Connecting,
    Connected,
}

/// Transport currently selected by Iroh for a connected device.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum ConnectionTransport {
    Direct,
    Relay,
    Custom,
    Unknown,
}

/// User-visible state for one trusted device.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct ManagedDevice {
    pub endpoint_id: String,
    pub device: DeviceInfo,
    pub status: DeviceStatus,
    pub transport: ConnectionTransport,
    pub selected: bool,
}

/// Kind of change published by the device manager.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum DeviceEventKind {
    Added,
    Changed,
    Removed,
}

/// Reactive update from the shared device manager.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct DeviceEvent {
    pub kind: DeviceEventKind,
    pub device: ManagedDevice,
}

/// Failure while waiting for device changes.
#[derive(Debug, Error, uniffi::Error)]
pub enum DeviceEventError {
    #[error("the Mist device event stream is closed")]
    Closed,
    #[error("device subscriber missed {count} events")]
    Lagged { count: u64 },
    #[error("failed to run device work on the Mist native executor: {reason}")]
    Executor { reason: String },
}

/// Failure while managing a trusted device.
#[derive(Debug, Error, uniffi::Error)]
pub enum DeviceError {
    #[error("invalid endpoint ID: {reason}")]
    InvalidEndpointId { reason: String },
    #[error("unknown paired device {endpoint_id}")]
    UnknownDevice { endpoint_id: String },
    #[error("device {endpoint_id} is disconnected")]
    Disconnected { endpoint_id: String },
    #[error("failed to connect device: {reason}")]
    Connect { reason: String },
    #[error("device protocol failed: {reason}")]
    Protocol { reason: String },
    #[error("device persistence failed: {reason}")]
    Storage { reason: String },
    #[error("failed to run device work on the Mist native executor: {reason}")]
    Executor { reason: String },
}

/// Event-driven stream of paired-device changes.
#[uniffi::export]
#[async_trait]
pub trait DeviceEventStream: Send + Sync + Debug {
    /// Suspends until the next change is available.
    async fn next(&self) -> Result<DeviceEvent, DeviceEventError>;
}

/// User-controllable paired-device operations.
///
/// Network addresses intentionally do not appear here. They are discovered,
/// synchronized, and persisted by the runtime.
#[uniffi::export]
#[async_trait]
pub trait DeviceManager: Send + Sync + Debug {
    /// Returns current snapshots of every trusted device.
    fn list(&self) -> Vec<ManagedDevice>;

    /// Returns a single trusted device, if known.
    ///
    /// # Errors
    ///
    /// Returns an error for a malformed or unknown endpoint identity.
    fn get(&self, endpoint_id: String) -> Result<ManagedDevice, DeviceError>;

    /// Creates an independent reactive event stream.
    fn subscribe(&self) -> Arc<dyn DeviceEventStream>;

    /// Changes whether feature sends target this device by default.
    ///
    /// # Errors
    ///
    /// Returns an error when the device is unknown or persistence fails.
    fn select(&self, endpoint_id: String, selected: bool) -> Result<(), DeviceError>;

    /// Establishes an authenticated session with a trusted device.
    async fn connect(&self, endpoint_id: String) -> Result<(), DeviceError>;

    /// Closes this device's active session without revoking trust.
    ///
    /// # Errors
    ///
    /// Returns an error when the device is unknown or already disconnected.
    fn disconnect(&self, endpoint_id: String) -> Result<(), DeviceError>;

    /// Revokes trust and removes all persisted metadata for this device.
    ///
    /// # Errors
    ///
    /// Returns an error when the device is unknown or persistence fails.
    fn forget(&self, endpoint_id: String) -> Result<(), DeviceError>;
}
