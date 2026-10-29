//! Shared native runtime for Mist.
//!
//! [`MistRuntime`] is the root of one running Mist instance. It owns the Iroh
//! endpoint, peer connections, and feature services such as
//! [`Pairing`]. The same runtime is exposed to mobile hosts through `UniFFI`.

mod device;
mod pairing;
mod protocol;
mod runtime;

pub use device::{
    DeviceError, DeviceInfo, DeviceStore, Devices, FileStore, MemoryStore, Peer, Platform,
};
pub use pairing::{Pairing, PairingError, PairingOffer};
pub use runtime::{MistBuilder, MistRuntime, Runtime, RuntimeError, create_mist_runtime};

/// Iroh application protocol negotiated by every Mist connection.
pub const MIST_ALPN: &[u8] = b"mist/1";

uniffi::setup_scaffolding!();
