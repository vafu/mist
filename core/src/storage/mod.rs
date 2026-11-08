//! Rust-owned persistent identity, trust, and address storage.

mod sqlite;

use std::fmt::Debug;
use std::sync::Arc;

use iroh::address_lookup::{AddressLookup, EndpointInfo, Item};
use iroh::{EndpointAddr, EndpointId, TransportAddr};
use mist_app_contract::DeviceInfo;
use n0_future::boxed::BoxStream;
use n0_future::stream::{self, StreamExt};
use thiserror::Error;

pub(crate) use sqlite::SqliteStateStore;

/// Why a transport address is trusted by the local runtime.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AddressSource {
    PairingOffer,
    AuthenticatedPeer,
}

impl AddressSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::PairingOffer => "pairing_offer",
            Self::AuthenticatedPeer => "authenticated_peer",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StoredDevice {
    pub(crate) endpoint_id: EndpointId,
    pub(crate) device: DeviceInfo,
    pub(crate) selected: bool,
}

#[derive(Debug, Error)]
pub(crate) enum StoreError {
    #[error("failed to create Mist storage directory: {0}")]
    CreateDirectory(#[source] std::io::Error),
    #[error("SQLite state operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("stored Mist identity is corrupt: {0}")]
    CorruptIdentity(String),
    #[error("stored device {endpoint_id} is corrupt: {reason}")]
    CorruptDevice { endpoint_id: String, reason: String },
    #[error("stored address for {endpoint_id} is corrupt: {reason}")]
    CorruptAddress { endpoint_id: String, reason: String },
}

pub(crate) trait StateStore: Send + Sync + Debug {
    fn load_identity(&self) -> Result<Option<[u8; 32]>, StoreError>;
    fn save_identity(&self, identity: &[u8; 32]) -> Result<(), StoreError>;
    fn load_devices(&self) -> Result<Vec<StoredDevice>, StoreError>;
    fn upsert_device(
        &self,
        endpoint_id: EndpointId,
        device: &DeviceInfo,
        selected_if_new: bool,
    ) -> Result<(), StoreError>;
    fn set_selected(&self, endpoint_id: EndpointId, selected: bool) -> Result<(), StoreError>;
    fn forget_device(&self, endpoint_id: EndpointId) -> Result<(), StoreError>;
    fn load_addresses(&self, endpoint_id: EndpointId) -> Result<Vec<TransportAddr>, StoreError>;
    fn load_addresses_from(
        &self,
        endpoint_id: EndpointId,
        source: AddressSource,
    ) -> Result<Vec<TransportAddr>, StoreError>;
    fn add_addresses(
        &self,
        endpoint_id: EndpointId,
        source: AddressSource,
        addresses: &[TransportAddr],
    ) -> Result<(), StoreError>;
    fn remove_addresses(
        &self,
        endpoint_id: EndpointId,
        source: AddressSource,
        addresses: &[TransportAddr],
    ) -> Result<(), StoreError>;
}

#[derive(Clone, Debug)]
pub(crate) struct StoredAddressLookup {
    store: Arc<dyn StateStore>,
}

#[cfg(test)]
mod test;

impl StoredAddressLookup {
    pub(crate) fn new(store: Arc<dyn StateStore>) -> Self {
        Self { store }
    }
}

impl AddressLookup for StoredAddressLookup {
    fn resolve(
        &self,
        endpoint_id: EndpointId,
    ) -> Option<BoxStream<Result<Item, iroh::address_lookup::Error>>> {
        match self.store.load_addresses(endpoint_id) {
            Ok(addresses) if addresses.is_empty() => None,
            Ok(addresses) => {
                let info = EndpointInfo::from(EndpointAddr::from_parts(endpoint_id, addresses));
                Some(stream::iter(Some(Ok(Item::new(info, "mist_state_store", None)))).boxed())
            }
            Err(error) => Some(
                stream::iter(Some(Err(iroh::address_lookup::Error::from_err(
                    "mist_state_store",
                    error,
                ))))
                .boxed(),
            ),
        }
    }
}
