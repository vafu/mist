use std::sync::Arc;

use iroh::address_lookup::AddressLookupBuilder;
use iroh::endpoint::Builder;
use iroh::endpoint::presets;
use iroh::endpoint::transports::CustomTransport;
use iroh::{Endpoint, RelayMode, SecretKey};

use super::{MistRuntime, RuntimeError, executor};
use crate::{DeviceInfo, DeviceStore, MIST_ALPN, MemoryStore};

/// Configures the Iroh endpoint owned by a [`MistRuntime`].
///
/// The default configuration uses Iroh's production relay and address lookup
/// services. Custom transports and address lookups must be added before
/// calling [`bind`](Self::bind).
#[derive(Debug)]
pub struct MistBuilder {
    endpoint: Builder,
    device_info: DeviceInfo,
    identity: Option<SecretKey>,
    store: Option<Arc<dyn DeviceStore>>,
}

impl MistBuilder {
    /// Creates a production endpoint configuration accepting the Mist ALPN.
    #[must_use]
    pub fn new(device_info: DeviceInfo) -> Self {
        Self {
            endpoint: Endpoint::builder(presets::N0).alpns(vec![MIST_ALPN.to_vec()]),
            device_info,
            identity: None,
            store: None,
        }
    }

    /// Uses an existing Iroh identity instead of generating a new one.
    #[must_use]
    pub fn secret_key(mut self, secret_key: SecretKey) -> Self {
        self.identity = Some(secret_key);
        self
    }

    /// Uses persistent or test storage for identity and trusted peer records.
    /// Without this option, the builder uses isolated ephemeral storage.
    #[must_use]
    pub fn store(mut self, store: Arc<dyn DeviceStore>) -> Self {
        self.store = Some(store);
        self
    }

    /// Disables relays and built-in address lookups.
    ///
    /// This is useful for local tests and fully platform-provided networking.
    #[must_use]
    pub fn local_only(mut self) -> Self {
        self.endpoint = self
            .endpoint
            .clear_address_lookup()
            .relay_mode(RelayMode::Disabled);
        self
    }

    /// Adds a platform-provided packet transport to the Iroh endpoint.
    #[must_use]
    pub fn custom_transport(mut self, transport: Arc<dyn CustomTransport>) -> Self {
        self.endpoint = self.endpoint.add_custom_transport(transport);
        self
    }

    /// Adds an address lookup or publication service to the Iroh endpoint.
    #[must_use]
    pub fn address_lookup(mut self, lookup: impl AddressLookupBuilder) -> Self {
        self.endpoint = self.endpoint.address_lookup(lookup);
        self
    }

    /// Binds the endpoint and constructs the stable runtime root.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::Bind`] when Iroh cannot bind the configured
    /// endpoint.
    pub async fn bind(self) -> Result<Arc<MistRuntime>, RuntimeError> {
        executor::run(async move {
            let storage_error = |error: crate::DeviceError| RuntimeError::Storage {
                reason: error.to_string(),
            };
            let store = if let Some(store) = self.store {
                store
            } else {
                let identity = if let Some(identity) = &self.identity {
                    identity.clone()
                } else {
                    let mut bytes = [0; 32];
                    getrandom::fill(&mut bytes).map_err(|error| RuntimeError::Storage {
                        reason: format!("generate identity: {error}"),
                    })?;
                    SecretKey::from_bytes(&bytes)
                };
                Arc::new(MemoryStore::new(identity)) as Arc<dyn DeviceStore>
            };
            let identity = store.identity().map_err(storage_error)?;
            store.peers().map_err(storage_error)?;
            if let Some(explicit) = self.identity
                && explicit.public() != identity.public()
            {
                return Err(RuntimeError::Storage {
                    reason: "explicit key differs from stored identity".to_owned(),
                });
            }
            let endpoint = self
                .endpoint
                .secret_key(identity)
                .bind()
                .await
                .map_err(|error| RuntimeError::Bind {
                    reason: error.to_string(),
                })?;
            Ok(MistRuntime::from_endpoint(
                endpoint,
                self.device_info,
                store,
            ))
        })
        .await?
    }
}
