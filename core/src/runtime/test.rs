use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use iroh::endpoint::Connection;
use iroh::{EndpointAddr, EndpointId, SecretKey};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::{Connections, RuntimeError};
use crate::pairing::IrohPairing;
use crate::{DeviceInfo, Pairing, PairingError, Platform};

#[derive(Debug, Default)]
struct UnavailableConnections {
    dial_attempts: AtomicUsize,
}

#[async_trait]
impl Connections for UnavailableConnections {
    fn address(&self) -> EndpointAddr {
        EndpointAddr::new(SecretKey::from_bytes(&[7; 32]).public())
    }

    async fn dial(&self, _peer: EndpointAddr) -> Result<Connection, RuntimeError> {
        self.dial_attempts.fetch_add(1, Ordering::SeqCst);
        Err(RuntimeError::Closed)
    }

    async fn connect(&self, _peer: EndpointAddr) -> Result<Connection, RuntimeError> {
        Err(RuntimeError::Closed)
    }

    async fn run(self: Arc<Self>, _pairing: Arc<dyn crate::pairing::IncomingPairing>) {}

    fn register(&self, _connection: Connection) -> Result<Connection, RuntimeError> {
        panic!("failed authorization must not register a connection")
    }
    fn trust(
        &self,
        _peer: crate::Peer,
        _connection: Connection,
        _cancelled: &CancellationToken,
    ) -> Result<Connection, RuntimeError> {
        panic!("failed authorization must not save trust")
    }
    fn forget(&self, _peer: EndpointId) -> Result<(), crate::DeviceError> {
        Ok(())
    }

    fn get(&self, _peer: EndpointId) -> Option<Connection> {
        None
    }

    fn is_connected(&self) -> bool {
        false
    }

    fn close(&self) {}
}

#[tokio::test]
async fn pairing_uses_injected_connection_behavior_without_a_session() {
    let connections = Arc::new(UnavailableConnections::default());
    let pairing: Arc<dyn Pairing> = Arc::new(IrohPairing::new(
        connections.clone(),
        DeviceInfo {
            display_name: "local device".to_owned(),
            platform: Platform::Linux,
        },
        CancellationToken::new(),
        TaskTracker::new(),
    ));

    let offer = pairing.initiate().unwrap();
    assert!(matches!(
        pairing.accept(offer).await,
        Err(PairingError::Connect { .. })
    ));
    assert_eq!(connections.dial_attempts.load(Ordering::SeqCst), 1);
}
