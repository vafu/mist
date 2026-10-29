//! Device identity metadata exchanged after pairing authorization.

mod file;
mod memory;
mod service;

use std::fmt::Debug;

use iroh::{EndpointId, SecretKey};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use file::FileStore;
pub use memory::MemoryStore;
pub(crate) use service::DeviceService;
use service::validate_peer;

/// The operating-system family hosting Mist.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, uniffi::Enum)]
pub enum Platform {
    /// Android.
    Android,
    /// Linux.
    Linux,
    /// Another operating-system family.
    Other,
}

/// User-visible information identifying a device.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct DeviceInfo {
    /// The device name displayed to its paired peers.
    pub display_name: String,
    /// The operating-system family hosting Mist.
    pub platform: Platform,
}

/// A remote peer's endpoint identity and device metadata.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct Peer {
    /// The peer's Iroh endpoint identity.
    pub endpoint_id: String,
    /// Device metadata received from the authorized peer.
    pub device: DeviceInfo,
}

/// Persistence required by local identity and pairing trust.
///
/// Implementations serialize updates and distinguish missing state from corrupt
/// or unreadable state. `save` replaces a record by endpoint ID; it never creates
/// a second identity for the same peer. One store can be shared by instances in
/// tests; production installations use distinct state directories.
pub trait DeviceStore: Send + Sync + Debug {
    /// Loads or creates the local identity, never replacing unreadable state.
    ///
    /// # Errors
    /// Returns a storage or data-validation failure.
    fn identity(&self) -> Result<SecretKey, DeviceError>;
    /// Lists trusted peer records.
    ///
    /// # Errors
    /// Returns a storage or data-validation failure.
    fn peers(&self) -> Result<Vec<Peer>, DeviceError>;
    /// Looks up trust by authenticated endpoint identity.
    ///
    /// # Errors
    /// Returns a storage or data-validation failure.
    fn get(&self, id: EndpointId) -> Result<Option<Peer>, DeviceError>;
    /// Persists an authorized peer's metadata.
    ///
    /// # Errors
    /// Returns a storage or data-validation failure.
    fn save(&self, peer: Peer) -> Result<(), DeviceError>;
    /// Removes local trust; absent records are harmless.
    ///
    /// # Errors
    /// Returns a storage failure without reporting successful removal.
    fn remove(&self, id: EndpointId) -> Result<(), DeviceError>;
}

/// Shared core operations on saved devices, independent of connectivity.
#[uniffi::export]
pub trait Devices: Send + Sync + Debug {
    /// Lists saved peers, including offline devices.
    ///
    /// # Errors
    /// Returns a storage failure or closed-runtime error.
    fn peers(&self) -> Result<Vec<Peer>, DeviceError>;
    /// Removes local trust, invalidates invitations and closes this peer.
    ///
    /// This does not promise that an offline remote device removes its copy.
    ///
    /// # Errors
    /// Returns an invalid identity, storage failure or closed-runtime error.
    fn forget(&self, endpoint_id: String) -> Result<(), DeviceError>;
}

/// Identity/device persistence or operation failure.
#[derive(Clone, Debug, Error, uniffi::Error)]
pub enum DeviceError {
    /// The runtime no longer admits operations.
    #[error("the Mist runtime closed")]
    Closed,
    /// An operation on the state directory failed.
    #[error("device storage failed: {reason}")]
    Io {
        /// Filesystem diagnostic and operation context.
        reason: String,
    },
    /// Existing state or an endpoint identity could not be decoded.
    #[error("invalid device state: {reason}")]
    InvalidData {
        /// Validation diagnostic.
        reason: String,
    },
}

#[cfg(test)]
mod test;
