//! Host-only runtime construction and platform integration.
//!
//! Applications use [`mist_app_contract::MistApp`] after construction. This
//! module is intentionally the only public seam that knows about Iroh host
//! extensions or the on-disk state location.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use iroh::SecretKey;
use iroh::address_lookup::AddressLookupBuilder;
use iroh::endpoint::transports::CustomTransport;
use mist_app_contract::{DeviceInfo, MistApp};
use thiserror::Error;

use crate::runtime::{MistBuilder, executor};
use crate::storage::{SqliteStateStore, StateStore, StoredAddressLookup};

/// Failure while opening persisted state or binding the native runtime.
#[derive(Debug, Error, uniffi::Error)]
pub enum StartupError {
    /// Rust's binding executor could not run the construction task.
    #[error("failed to run the Mist native executor: {reason}")]
    Executor { reason: String },
    /// Persisted state could not be opened or validated.
    #[error("failed to open Mist state: {reason}")]
    Storage { reason: String },
    /// Iroh could not bind the local endpoint.
    #[error("failed to bind the Mist runtime: {reason}")]
    Bind { reason: String },
}

pub use crate::media::{LocalPlayer, MediaProvider, MediaProviderError, MediaWatcher};
pub use mist_app_contract::MediaCommandHandler;

/// Host composition builder for a persistent Mist runtime.
///
/// Custom transports and address lookups belong here because they configure
/// Iroh before binding; they are not part of the user-facing app contract.
#[derive(Debug)]
pub struct MistFactory {
    builder: MistBuilder,
    media: Option<Arc<dyn MediaProvider>>,
    media_handler: Option<Arc<dyn MediaCommandHandler>>,
}

impl MistFactory {
    /// Opens `mist.sqlite3` below `storage_directory`, loading or creating the
    /// stable local Iroh identity.
    ///
    /// # Errors
    ///
    /// Existing corrupt identity or database state fails startup rather than
    /// silently rotating the device identity.
    pub fn new(
        storage_directory: impl AsRef<Path>,
        device_info: DeviceInfo,
    ) -> Result<Self, StartupError> {
        let database_path = storage_directory.as_ref().join("mist.sqlite3");
        let store: Arc<dyn StateStore> =
            Arc::new(SqliteStateStore::open(&database_path).map_err(|error| {
                StartupError::Storage {
                    reason: error.to_string(),
                }
            })?);
        let identity = if let Some(identity) =
            store
                .load_identity()
                .map_err(|error| StartupError::Storage {
                    reason: error.to_string(),
                })? {
            identity
        } else {
            let identity = SecretKey::generate().to_bytes();
            store
                .save_identity(&identity)
                .map_err(|error| StartupError::Storage {
                    reason: error.to_string(),
                })?;
            identity
        };
        let lookup = StoredAddressLookup::new(Arc::clone(&store));
        Ok(Self {
            builder: MistBuilder::new(device_info)
                .secret_key(SecretKey::from_bytes(&identity))
                .address_lookup(lookup)
                .state_store(store)
                .blob_directory(storage_directory.as_ref().join("clipboard")),
            media: None,
            media_handler: None,
        })
    }

    /// Adds a platform packet transport, such as Bluetooth.
    #[must_use]
    pub fn custom_transport(mut self, transport: Arc<dyn CustomTransport>) -> Self {
        self.builder = self.builder.custom_transport(transport);
        self
    }

    /// Adds an additional Iroh address lookup or publisher.
    #[must_use]
    pub fn address_lookup(mut self, lookup: impl AddressLookupBuilder) -> Self {
        self.builder = self.builder.address_lookup(lookup);
        self
    }

    /// Disables relay networking for isolated tests.
    #[must_use]
    pub fn local_only(mut self) -> Self {
        self.builder = self.builder.local_only();
        self
    }

    /// Attaches the platform media backend.
    #[must_use]
    pub fn media(mut self, provider: Arc<dyn MediaProvider>) -> Self {
        self.media = Some(provider);
        self
    }

    /// Registers a callback handler for inbound media commands.
    ///
    /// Used by hosts that push local state through generated bindings instead of
    /// a native provider, such as Android.
    #[must_use]
    pub fn media_command_handler(mut self, handler: Arc<dyn MediaCommandHandler>) -> Self {
        self.media_handler = Some(handler);
        self
    }

    /// Binds the endpoint and returns only the stable application contract.
    ///
    /// # Errors
    ///
    /// Returns an error if Iroh or persisted state cannot initialize.
    pub async fn create(self) -> Result<Arc<dyn MistApp>, StartupError> {
        self.builder
            .bind()
            .await
            .map(|runtime| {
                if let Some(provider) = self.media {
                    runtime.attach_media(&provider);
                }
                if let Some(handler) = self.media_handler {
                    runtime.set_media_command_handler(handler);
                }
                runtime as Arc<dyn MistApp>
            })
            .map_err(|error| StartupError::Bind {
                reason: error.to_string(),
            })
    }
}

/// Creates the default persistent runtime for generated mobile bindings.
///
/// The platform passes an app-private directory. The database and cryptographic
/// identity remain entirely Rust-owned.
///
/// # Errors
///
/// Returns an error when the native executor, persistent state, or Iroh endpoint
/// cannot initialize.
#[uniffi::export]
pub async fn create_mist_app(
    storage_directory: String,
    device_info: DeviceInfo,
) -> Result<Arc<dyn MistApp>, StartupError> {
    executor::run(async move {
        MistFactory::new(PathBuf::from(storage_directory), device_info)?
            .create()
            .await
    })
    .await
    .map_err(|reason| StartupError::Executor { reason })?
}

/// Creates the default persistent runtime with a media command handler.
///
/// Android pushes local playback state through [`Media::publish_local`] and
/// receives transport commands through `handler`. Linux uses
/// [`MistFactory::media`] with a native MPRIS provider instead.
///
/// # Errors
///
/// Returns an error when the native executor, persistent state, or Iroh endpoint
/// cannot initialize.
#[uniffi::export]
pub async fn create_mist_app_with_media(
    storage_directory: String,
    device_info: DeviceInfo,
    handler: Box<dyn MediaCommandHandler>,
) -> Result<Arc<dyn MistApp>, StartupError> {
    let handler: Arc<dyn MediaCommandHandler> = Arc::from(handler);
    executor::run(async move {
        MistFactory::new(PathBuf::from(storage_directory), device_info)?
            .media_command_handler(handler)
            .create()
            .await
    })
    .await
    .map_err(|reason| StartupError::Executor { reason })?
}
