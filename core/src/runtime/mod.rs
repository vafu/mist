//! Runtime contracts, construction, and domain errors.

mod builder;
mod connections;
mod executor;
mod service;

pub(crate) use executor::run_owned;

use std::fmt::Debug;
use std::sync::Arc;

use async_trait::async_trait;
use iroh::endpoint::Connection;
use iroh::{EndpointAddr, EndpointId};
use thiserror::Error;

use crate::{DeviceError, Devices, Pairing, Peer};
use tokio_util::sync::CancellationToken;

pub use builder::MistBuilder;
pub use service::{MistRuntime, create_mist_runtime};

/// Behavior of one running Mist instance, shared by all hosts.
///
/// The runtime owns local networking resources; connections are addressed by
/// remote endpoint identity. Scheduling is an implementation detail, never a
/// dependency retrieved by a consumer. Concrete construction belongs to
/// [`MistBuilder`].
#[async_trait]
pub trait Runtime: Send + Sync + Debug {
    /// Returns the local endpoint address used to reach this instance.
    fn address(&self) -> EndpointAddr;

    /// Returns the pairing service for this instance.
    fn pairing(&self) -> Arc<dyn Pairing>;

    /// Returns saved-device operations over this same core instance.
    fn devices(&self) -> Arc<dyn Devices>;

    /// Looks up an active connection to a particular remote peer.
    fn connection(&self, peer: EndpointId) -> Option<Connection>;

    /// Reports whether at least one registered peer connection is open.
    fn is_connected(&self) -> bool;

    /// Opens a connection, exchanges the Mist hello, and registers the peer.
    ///
    /// # Errors
    /// Returns a transport connection error or an endpoint shutdown error.
    async fn connect(&self, peer: EndpointAddr) -> Result<Connection, RuntimeError>;

    /// Resolves a saved peer by endpoint ID using configured Iroh address lookup.
    ///
    /// # Errors
    /// Returns invalid identity, missing trust, transport, storage or closed errors.
    async fn reconnect(&self, endpoint_id: String) -> Result<(), RuntimeError>;

    /// Closes this instance's peer connections and endpoint.
    ///
    /// Shutdown must not stop an executor shared by other instances. This
    /// rejects new work, invalidates invitations, and waits for instance-owned
    /// operations to stop before returning. Repeated shutdown is harmless.
    ///
    /// # Errors
    /// Returns an executor failure if shutdown could not complete.
    async fn shutdown(&self) -> Result<(), RuntimeError>;
}

/// Transport operations needed by runtime ownership and pairing authorization.
///
/// Dialed/accepted candidates are not registered until their owner completes
/// the required handshake. Metadata and execution handles do not belong here.
/// Incoming acceptance ownership is internal to the core, not an app concern.
#[async_trait]
pub(crate) trait Connections: Send + Sync + Debug {
    fn address(&self) -> EndpointAddr;
    async fn dial(&self, peer: EndpointAddr) -> Result<Connection, RuntimeError>;
    async fn connect(&self, peer: EndpointAddr) -> Result<Connection, RuntimeError>;
    async fn run(self: Arc<Self>, pairing: Arc<dyn crate::pairing::IncomingPairing>);
    /// Chooses the same live connection at both ends using TLS-exported rank.
    fn register(&self, connection: Connection) -> Result<Connection, RuntimeError>;
    fn trust(
        &self,
        peer: Peer,
        connection: Connection,
        cancelled: &CancellationToken,
    ) -> Result<Connection, RuntimeError>;
    fn forget(&self, peer: EndpointId) -> Result<(), DeviceError>;
    fn get(&self, peer: EndpointId) -> Option<Connection>;
    fn is_connected(&self) -> bool;
    fn close(&self);
}

/// Runtime failures shared by direct Rust calls and generated bindings.
#[derive(Clone, Debug, Error, uniffi::Error)]
pub enum RuntimeError {
    /// Identity or trusted-peer storage failed.
    #[error("device storage failed: {reason}")]
    Storage {
        /// Underlying storage diagnostic.
        reason: String,
    },
    /// A connection requires a new pairing invitation.
    #[error("peer is not trusted: {endpoint_id}")]
    Untrusted {
        /// Authenticated remote endpoint identity.
        endpoint_id: String,
    },
    /// A peer did not complete the Mist control handshake.
    #[error("Mist protocol failed: {reason}")]
    Protocol {
        /// Underlying handshake diagnostic.
        reason: String,
    },
    /// The endpoint or instance has closed.
    #[error("the Mist runtime closed")]
    Closed,
    /// An outgoing transport connection failed.
    #[error("failed to connect: {reason}")]
    Connect {
        /// Underlying transport diagnostic.
        reason: String,
    },
    /// Execution of a core operation failed.
    #[error("failed to run the Mist native executor: {reason}")]
    Executor {
        /// Human-readable executor failure.
        reason: String,
    },
    /// Iroh could not bind the local endpoint.
    #[error("failed to bind the Mist runtime: {reason}")]
    Bind {
        /// Human-readable bind failure.
        reason: String,
    },
}

#[cfg(test)]
mod test;
