use std::collections::BTreeMap;
use std::sync::Mutex;

use iroh::{EndpointId, SecretKey};

use super::{DeviceError, DeviceStore, Peer, validate_peer};

/// Ephemeral storage for isolated runtimes and contract tests.
pub struct MemoryStore {
    identity: SecretKey,
    peers: Mutex<BTreeMap<String, Peer>>,
}

impl std::fmt::Debug for MemoryStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MemoryStore")
            .finish_non_exhaustive()
    }
}

impl MemoryStore {
    /// Creates ephemeral storage using an explicitly supplied identity.
    #[must_use]
    pub fn new(identity: SecretKey) -> Self {
        Self {
            identity,
            peers: Mutex::new(BTreeMap::new()),
        }
    }
}

impl DeviceStore for MemoryStore {
    fn identity(&self) -> Result<SecretKey, DeviceError> {
        Ok(self.identity.clone())
    }
    fn peers(&self) -> Result<Vec<Peer>, DeviceError> {
        Ok(self
            .peers
            .lock()
            .expect("peer store poisoned")
            .values()
            .cloned()
            .collect())
    }
    fn get(&self, id: EndpointId) -> Result<Option<Peer>, DeviceError> {
        Ok(self
            .peers
            .lock()
            .expect("peer store poisoned")
            .get(&id.to_string())
            .cloned())
    }
    fn save(&self, peer: Peer) -> Result<(), DeviceError> {
        validate_peer(&peer)?;
        self.peers
            .lock()
            .expect("peer store poisoned")
            .insert(peer.endpoint_id.clone(), peer);
        Ok(())
    }
    fn remove(&self, id: EndpointId) -> Result<(), DeviceError> {
        self.peers
            .lock()
            .expect("peer store poisoned")
            .remove(&id.to_string());
        Ok(())
    }
}
