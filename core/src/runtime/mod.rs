//! Native runtime ownership and construction.

mod builder;
mod executor;

use std::sync::{Arc, OnceLock};

use crate::pairing::{IrohPairing, Pairing};
use crate::{DeviceInfo, MIST_ALPN};
use iroh::endpoint::{BindError, ConnectError, ConnectingError, Connection};
use iroh::{Endpoint, EndpointAddr};
use thiserror::Error;
use tokio::runtime::Handle;

pub use builder::MistBuilder;

/// Failure while constructing or connecting a Mist runtime.
#[derive(Debug, Error)]
pub enum MistError {
    /// The local Iroh endpoint could not be bound.
    #[error("failed to bind the Mist endpoint")]
    Bind(#[from] BindError),

    /// An outgoing connection could not be established.
    #[error("failed to connect to the Mist peer")]
    Connect(#[from] ConnectError),

    /// An incoming connection failed during its handshake.
    #[error("failed to accept the Mist peer")]
    Accept(#[from] ConnectingError),

    /// The endpoint closed while waiting for an incoming connection.
    #[error("the Mist endpoint closed before a peer connected")]
    EndpointClosed,

    /// The runtime was asked to connect to a different peer after binding its
    /// one stable connection.
    #[error("the Mist runtime is already connected to a different peer")]
    ConnectedToDifferentPeer,
}

/// Failure exposed while creating the default runtime through `UniFFI`.
#[derive(Debug, Error, uniffi::Error)]
pub enum RuntimeError {
    /// The Rust executor used by foreign-language bindings could not run the
    /// requested operation.
    #[error("failed to run the Mist native executor: {reason}")]
    Executor {
        /// Human-readable executor failure for diagnostics.
        reason: String,
    },

    /// Iroh could not bind the local endpoint.
    #[error("failed to bind the Mist runtime: {reason}")]
    Bind {
        /// Human-readable Iroh failure for diagnostics.
        reason: String,
    },
}

#[derive(Debug)]
pub(crate) struct Session {
    endpoint: Endpoint,
    device_info: DeviceInfo,
    connection: OnceLock<Connection>,
    executor: Handle,
}

impl Session {
    fn new(endpoint: Endpoint, device_info: DeviceInfo) -> Self {
        Self {
            endpoint,
            device_info,
            connection: OnceLock::new(),
            executor: Handle::current(),
        }
    }

    pub(crate) fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    pub(crate) fn connection(&self) -> Option<&Connection> {
        self.connection.get()
    }

    pub(crate) fn device_info(&self) -> &DeviceInfo {
        &self.device_info
    }

    pub(crate) fn executor(&self) -> &Handle {
        &self.executor
    }

    pub(crate) async fn connect_candidate(
        &self,
        peer: EndpointAddr,
    ) -> Result<Connection, MistError> {
        let expected_id = peer.id;
        if let Some(connection) = self.connection.get() {
            if connection.remote_id() == expected_id {
                return Ok(connection.clone());
            }
            return Err(MistError::ConnectedToDifferentPeer);
        }

        let connection = self.endpoint.connect(peer, MIST_ALPN).await?;
        if connection.remote_id() != expected_id {
            return Err(MistError::ConnectedToDifferentPeer);
        }
        Ok(connection)
    }

    pub(crate) async fn accept_candidate(&self) -> Result<Connection, MistError> {
        if let Some(connection) = self.connection.get() {
            return Ok(connection.clone());
        }
        let incoming = self
            .endpoint
            .accept()
            .await
            .ok_or(MistError::EndpointClosed)?;
        Ok(incoming.await?)
    }

    pub(crate) fn install(&self, connection: Connection) -> Result<Connection, MistError> {
        if let Some(existing) = self.connection.get() {
            return if existing.remote_id() == connection.remote_id() {
                Ok(existing.clone())
            } else {
                Err(MistError::ConnectedToDifferentPeer)
            };
        }

        if self.connection.set(connection.clone()).is_ok() {
            return Ok(connection);
        }

        let existing = self
            .connection
            .get()
            .expect("another thread initialized the connection");
        if existing.remote_id() == connection.remote_id() {
            Ok(existing.clone())
        } else {
            Err(MistError::ConnectedToDifferentPeer)
        }
    }

    pub(crate) async fn connect(&self, peer: EndpointAddr) -> Result<Connection, MistError> {
        let candidate = self.connect_candidate(peer).await?;
        self.install(candidate)
    }

    pub(crate) async fn accept(&self) -> Result<Connection, MistError> {
        let candidate = self.accept_candidate().await?;
        self.install(candidate)
    }

    async fn close(&self) {
        if let Some(connection) = self.connection.get() {
            connection.close(0_u32.into(), b"Mist runtime closed");
        }
        self.endpoint.close().await;
    }
}

/// Root object for one running Mist instance.
///
/// A runtime owns one Iroh endpoint and installs at most one stable peer
/// connection. `UniFFI` exposes the same Rust allocation to Kotlin as an object
/// proxy; it does not construct a second runtime.
#[derive(Debug, uniffi::Object)]
pub struct MistRuntime {
    session: Arc<Session>,
    pairing: Arc<dyn Pairing>,
}

impl MistRuntime {
    pub(crate) fn from_endpoint(endpoint: Endpoint, device_info: DeviceInfo) -> Arc<Self> {
        let session = Arc::new(Session::new(endpoint, device_info));
        let pairing = Arc::new(IrohPairing::new(Arc::clone(&session)));
        Arc::new(Self { session, pairing })
    }

    /// Returns the Iroh endpoint owned by this runtime.
    #[must_use]
    pub fn endpoint(&self) -> &Endpoint {
        self.session.endpoint()
    }

    /// Returns the runtime's stable connection after pairing completes.
    #[must_use]
    pub fn connection(&self) -> Option<&Connection> {
        self.session.connection()
    }

    /// Connects this runtime to a known peer outside of the pairing facade.
    ///
    /// This lower-level Rust API is used by the CLI bootstrap and tests.
    /// Mobile hosts should use [`Pairing::accept`].
    ///
    /// # Errors
    ///
    /// Returns an Iroh connection error, or
    /// [`MistError::ConnectedToDifferentPeer`] if another peer won the runtime's
    /// connection slot.
    pub async fn connect(&self, peer: impl Into<EndpointAddr>) -> Result<Connection, MistError> {
        self.session.connect(peer.into()).await
    }

    /// Accepts the next connection outside of the pairing facade.
    ///
    /// # Errors
    ///
    /// Returns an Iroh handshake error or [`MistError::EndpointClosed`].
    pub async fn accept(&self) -> Result<Connection, MistError> {
        self.session.accept().await
    }
}

#[uniffi::export]
impl MistRuntime {
    /// Returns this runtime's pairing service.
    #[must_use]
    pub fn pairing(&self) -> Arc<dyn Pairing> {
        Arc::clone(&self.pairing)
    }

    /// Returns the local Iroh endpoint identity.
    ///
    /// The iroh-kmp endpoint wrapper will replace this convenience method once
    /// native handle sharing is wired.
    #[must_use]
    pub fn endpoint_id(&self) -> String {
        self.session.endpoint.id().to_string()
    }

    /// Reports whether pairing has installed the runtime's connection.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.session.connection.get().is_some()
    }

    /// Shuts down the stable connection, if present, and then the endpoint.
    pub async fn shutdown(&self) {
        let session = Arc::clone(&self.session);
        let executor = session.executor().clone();
        let _ = executor.spawn(async move { session.close().await }).await;
    }
}

/// Creates the default production runtime for mobile bindings.
///
/// Platform capabilities and custom Iroh transports will become additional
/// parameters of the mobile composition factory.
///
/// # Errors
///
/// Returns [`RuntimeError::Bind`] if Iroh cannot bind the endpoint.
#[uniffi::export]
pub async fn create_mist_runtime(
    device_info: DeviceInfo,
) -> Result<Arc<MistRuntime>, RuntimeError> {
    executor::run(async move {
        MistBuilder::new(device_info)
            .bind()
            .await
            .map_err(|error| RuntimeError::Bind {
                reason: error.to_string(),
            })
    })
    .await
    .map_err(|reason| RuntimeError::Executor { reason })?
}
