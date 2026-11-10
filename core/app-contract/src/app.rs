use std::fmt::Debug;
use std::sync::Arc;

use async_trait::async_trait;

use crate::{Clipboard, DeviceManager, FileTransfer, Media, NotificationChannel, Pairing};

/// Root application contract for one running Mist instance.
///
/// Hosts retain this object for the service lifetime. Feature objects returned
/// by it operate on the same Rust-owned endpoint and persisted device state.
#[uniffi::export]
#[async_trait]
pub trait MistApp: Send + Sync + Debug {
    /// Returns the stable local Iroh endpoint identity.
    fn endpoint_id(&self) -> String;

    /// Returns the paired-device management service.
    fn device_manager(&self) -> Arc<dyn DeviceManager>;

    /// Returns the pairing service.
    fn pairing(&self) -> Arc<dyn Pairing>;

    /// Returns the bidirectional notification service.
    fn notification_channel(&self) -> Arc<dyn NotificationChannel>;

    /// Returns the clipboard sharing service.
    fn clipboard(&self) -> Arc<dyn Clipboard>;

    /// Returns the media player service.
    fn media(&self) -> Arc<dyn Media>;

    /// Returns the file transfer service.
    fn files(&self) -> Arc<dyn FileTransfer>;

    /// Closes active sessions and the local endpoint.
    async fn shutdown(&self);
}
