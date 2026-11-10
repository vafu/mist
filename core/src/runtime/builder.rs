use std::sync::Arc;

use iroh::address_lookup::AddressLookupBuilder;
use iroh::endpoint::Builder;
use iroh::endpoint::presets;
use iroh::endpoint::transports::CustomTransport;
use iroh::{Endpoint, RelayMode, SecretKey};

use super::address_publisher::{LocalAddressPublisher, PublishedAddresses};
use super::{MistError, MistRuntime};
use crate::storage::{SqliteStateStore, StateStore};
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
    store: Arc<dyn StateStore>,
    local_addresses: tokio::sync::watch::Receiver<Option<PublishedAddresses>>,
    blob_directory: std::path::PathBuf,
    file_store_directory: std::path::PathBuf,
    file_inbox_directory: Option<std::path::PathBuf>,
}

impl MistBuilder {
    /// Creates a production endpoint configuration accepting the Mist ALPN.
    #[must_use]
    pub fn new(device_info: DeviceInfo) -> Self {
        let (publisher, local_addresses) = LocalAddressPublisher::new();
        Self {
            endpoint: Endpoint::builder(presets::N0)
                .alpns(vec![MIST_ALPN.to_vec()])
                .address_lookup(publisher),
            device_info,
            store: Arc::new(
                SqliteStateStore::in_memory().expect("in-memory SQLite initialization must work"),
            ),
            local_addresses,
            blob_directory: std::env::temp_dir().join("mist-clipboard"),
            file_store_directory: unique_temp_directory("mist-files"),
            file_inbox_directory: None,
        }
    }

    /// Sets the app-private directory used for received clipboard blobs.
    #[must_use]
    pub fn blob_directory(mut self, directory: impl Into<std::path::PathBuf>) -> Self {
        self.blob_directory = directory.into();
        self
    }

    /// Sets the app-private directory used for transferred file blobs.
    #[must_use]
    pub fn file_store_directory(mut self, directory: impl Into<std::path::PathBuf>) -> Self {
        self.file_store_directory = directory.into();
        self
    }

    /// Sets the directory where accepted incoming files are stored.
    ///
    /// Without an inbox, incoming offers can be listed but not accepted.
    #[must_use]
    pub fn file_inbox_directory(mut self, directory: impl Into<std::path::PathBuf>) -> Self {
        self.file_inbox_directory = Some(directory.into());
        self
    }

    pub(crate) fn state_store(mut self, store: Arc<dyn StateStore>) -> Self {
        self.store = store;
        self
    }

    /// Uses an existing Iroh identity instead of generating a new one.
    #[must_use]
    pub fn secret_key(mut self, secret_key: SecretKey) -> Self {
        self.endpoint = self.endpoint.secret_key(secret_key);
        self
    }

    /// Disables relay transport while preserving configured address lookups.
    ///
    /// This is useful for local tests and fully platform-provided networking.
    #[must_use]
    pub fn local_only(mut self) -> Self {
        self.endpoint = self.endpoint.relay_mode(RelayMode::Disabled);
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
        let endpoint = self
            .endpoint
            .alpns(vec![MIST_ALPN.to_vec(), iroh_blobs::ALPN.to_vec()])
            .bind()
            .await?;
        let authorization = crate::transfer::ShareAuthorization::default();
        let blob_store = crate::transfer::IrohBlobStore::load(
            self.file_store_directory,
            &endpoint,
            authorization,
        )
        .await
        .map_err(|error| MistError::Transfer {
            reason: error.to_string(),
        })?;
        MistRuntime::from_endpoint(
            endpoint,
            &self.device_info,
            self.store,
            self.local_addresses,
            self.blob_directory,
            std::sync::Arc::new(blob_store),
            self.file_inbox_directory,
        )
        .map_err(MistError::from)
    }
}

fn unique_temp_directory(prefix: &str) -> std::path::PathBuf {
    let mut random = [0_u8; 16];
    let suffix = if getrandom::fill(&mut random).is_ok() {
        let mut suffix = String::with_capacity(random.len() * 2);
        for byte in random {
            use std::fmt::Write as _;
            let _ = write!(&mut suffix, "{byte:02x}");
        }
        suffix
    } else {
        std::process::id().to_string()
    };
    std::env::temp_dir().join(format!("{prefix}-{suffix}"))
}
