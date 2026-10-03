use std::sync::Arc;

use iroh::address_lookup::AddressLookupBuilder;
use iroh::endpoint::Builder;
use iroh::endpoint::presets;
use iroh::endpoint::transports::CustomTransport;
use iroh::{Endpoint, RelayMode, SecretKey};

use super::{MistError, MistRuntime};
use crate::{DeviceInfo, MIST_ALPN};

/// Configures the Iroh endpoint owned by a [`MistRuntime`].
///
/// The default configuration uses Iroh's production relay and address lookup
/// services. Custom transports and address lookups must be added before
/// calling [`bind`](Self::bind).
#[derive(Debug)]
pub struct MistBuilder {
    endpoint: Builder,
    device_info: DeviceInfo,
}

impl MistBuilder {
    /// Creates a production endpoint configuration accepting the Mist ALPN.
    #[must_use]
    pub fn new(device_info: DeviceInfo) -> Self {
        Self {
            endpoint: Endpoint::builder(presets::N0).alpns(vec![MIST_ALPN.to_vec()]),
            device_info,
        }
    }

    /// Uses an existing Iroh identity instead of generating a new one.
    #[must_use]
    pub fn secret_key(mut self, secret_key: SecretKey) -> Self {
        self.endpoint = self.endpoint.secret_key(secret_key);
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
    /// Returns [`MistError::Bind`] when Iroh cannot bind the configured
    /// endpoint.
    pub async fn bind(self) -> Result<Arc<MistRuntime>, MistError> {
        let endpoint = self.endpoint.bind().await?;
        Ok(MistRuntime::from_endpoint(endpoint, self.device_info))
    }
}
