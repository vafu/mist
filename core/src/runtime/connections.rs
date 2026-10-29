use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointAddr, EndpointId};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::{Connections, RuntimeError};
use crate::pairing::IncomingPairing;
use crate::protocol::wire::{ControlFrame, control_frame};
use crate::protocol::{
    decode_session_hello, read_control_frame, session_hello, write_control_frame,
};
use crate::{DeviceError, DeviceInfo, DeviceStore, MIST_ALPN, Peer};

#[derive(Debug)]
pub(super) struct IrohConnections {
    endpoint: Endpoint,
    local_device: DeviceInfo,
    peers: Mutex<HashMap<EndpointId, (Connection, [u8; 32])>>,
    shutdown: CancellationToken,
    tasks: TaskTracker,
    store: Arc<dyn DeviceStore>,
}

impl IrohConnections {
    pub(super) fn new(
        endpoint: Endpoint,
        local_device: DeviceInfo,
        shutdown: CancellationToken,
        tasks: TaskTracker,
        store: Arc<dyn DeviceStore>,
    ) -> Self {
        Self {
            endpoint,
            local_device,
            peers: Mutex::new(HashMap::new()),
            shutdown,
            tasks,
            store,
        }
    }

    fn insert(
        &self,
        peers: &mut HashMap<EndpointId, (Connection, [u8; 32])>,
        connection: Connection,
    ) -> Result<Connection, RuntimeError> {
        let mut rank = [0; 32];
        connection
            .export_keying_material(&mut rank, b"mist/connection-priority", b"")
            .map_err(|error| Self::protocol(format!("{error:?}")))?;
        if self.shutdown.is_cancelled() {
            connection.close(0_u32.into(), b"runtime closed");
            return Err(RuntimeError::Closed);
        }
        if let Some((current, current_rank)) = peers.get(&connection.remote_id())
            && current.close_reason().is_none()
        {
            if *current_rank <= rank {
                if *current_rank != rank {
                    connection.close(2_u32.into(), b"duplicate connection");
                }
                return Ok(current.clone());
            }
            current.close(2_u32.into(), b"duplicate connection");
        }
        peers.insert(connection.remote_id(), (connection.clone(), rank));
        Ok(connection)
    }

    fn protocol(error: impl std::fmt::Display) -> RuntimeError {
        RuntimeError::Protocol {
            reason: error.to_string(),
        }
    }

    async fn receive(
        &self,
        connection: &Connection,
        pairing: &dyn IncomingPairing,
    ) -> Result<(), RuntimeError> {
        let (mut send, mut recv) = connection.accept_bi().await.map_err(Self::protocol)?;
        let frame = read_control_frame(&mut recv)
            .await
            .map_err(Self::protocol)?;
        match frame.body {
            Some(control_frame::Body::PairingAuthenticate(authentication)) => pairing
                .authorize(connection.clone(), send, recv, authentication.pairing_token)
                .await
                .map_err(Self::protocol),
            Some(control_frame::Body::SessionHello(hello)) => {
                decode_session_hello(&hello).map_err(Self::protocol)?;
                self.register(connection.clone())?;
                write_control_frame(
                    &mut send,
                    &ControlFrame {
                        body: Some(control_frame::Body::SessionHello(session_hello(
                            &self.local_device,
                        ))),
                    },
                )
                .await
                .map_err(Self::protocol)?;
                send.finish().map_err(Self::protocol)?;
                Ok(())
            }
            _ => Err(Self::protocol(
                "expected pairing authorization or session hello",
            )),
        }
    }
}

#[async_trait]
impl Connections for IrohConnections {
    fn address(&self) -> EndpointAddr {
        self.endpoint.addr()
    }

    async fn dial(&self, peer: EndpointAddr) -> Result<Connection, RuntimeError> {
        if self.shutdown.is_cancelled() {
            return Err(RuntimeError::Closed);
        }
        self.endpoint
            .connect(peer, MIST_ALPN)
            .await
            .map_err(|error| RuntimeError::Connect {
                reason: error.to_string(),
            })
    }

    async fn connect(&self, peer: EndpointAddr) -> Result<Connection, RuntimeError> {
        if self.shutdown.is_cancelled() {
            return Err(RuntimeError::Closed);
        }
        if self
            .store
            .get(peer.id)
            .map_err(RuntimeError::from)?
            .is_none()
        {
            return Err(RuntimeError::Untrusted {
                endpoint_id: peer.id.to_string(),
            });
        }
        if let Some(connection) = self.get(peer.id) {
            return Ok(connection);
        }
        let connection = self.dial(peer).await?;
        let result = async {
            let (mut send, mut recv) = connection.open_bi().await.map_err(Self::protocol)?;
            write_control_frame(
                &mut send,
                &ControlFrame {
                    body: Some(control_frame::Body::SessionHello(session_hello(
                        &self.local_device,
                    ))),
                },
            )
            .await
            .map_err(Self::protocol)?;
            let response = read_control_frame(&mut recv)
                .await
                .map_err(Self::protocol)?;
            match response.body {
                Some(control_frame::Body::SessionHello(hello)) => {
                    decode_session_hello(&hello).map_err(Self::protocol)?;
                }
                _ => return Err(Self::protocol("expected session hello")),
            }
            send.finish().map_err(Self::protocol)?;
            self.register(connection.clone())
        }
        .await;
        match result {
            Ok(connection) => Ok(connection),
            Err(error) => {
                connection.close(1_u32.into(), b"handshake failed");
                self.get(connection.remote_id()).ok_or(error)
            }
        }
    }

    async fn run(self: Arc<Self>, pairing: Arc<dyn IncomingPairing>) {
        loop {
            let incoming = tokio::select! {
                biased;
                () = self.shutdown.cancelled() => return,
                incoming = self.endpoint.accept() => match incoming { Some(incoming) => incoming, None => return },
            };
            let connections = Arc::clone(&self);
            let pairing = Arc::clone(&pairing);
            self.tasks.spawn(async move {
                let work = async {
                    if let Ok(connection) = incoming.await {
                        if connections
                            .receive(&connection, pairing.as_ref())
                            .await
                            .is_err()
                        {
                            connection.close(1_u32.into(), b"handshake rejected");
                        } else {
                            // A temporary re-pairing transport may not be the
                            // registered winner. Retain it until its initiator
                            // reads the final FIN and closes it explicitly.
                            connection.closed().await;
                        }
                    }
                };
                tokio::select! {
                    biased;
                    () = connections.shutdown.cancelled() => {},
                    () = work => {},
                }
            });
        }
    }

    fn register(&self, connection: Connection) -> Result<Connection, RuntimeError> {
        let mut peers = self.peers.lock().expect("connection registry poisoned");
        if self
            .store
            .get(connection.remote_id())
            .map_err(RuntimeError::from)?
            .is_none()
        {
            return Err(RuntimeError::Untrusted {
                endpoint_id: connection.remote_id().to_string(),
            });
        }
        self.insert(&mut peers, connection)
    }

    fn trust(
        &self,
        peer: Peer,
        connection: Connection,
        cancelled: &CancellationToken,
    ) -> Result<Connection, RuntimeError> {
        let mut peers = self.peers.lock().expect("connection registry poisoned");
        if cancelled.is_cancelled() || self.shutdown.is_cancelled() {
            return Err(RuntimeError::Closed);
        }
        self.store.save(peer).map_err(RuntimeError::from)?;
        // Re-pairing refreshes trust without replacing a working transport.
        // The invitation holder closes its temporary candidate after observing
        // the creator's stream FIN, so metadata is not cut off mid-handshake.
        if let Some((current, _)) = peers.get(&connection.remote_id())
            && current.close_reason().is_none()
        {
            return Ok(current.clone());
        }
        self.insert(&mut peers, connection)
    }

    fn forget(&self, peer: EndpointId) -> Result<(), DeviceError> {
        let mut peers = self.peers.lock().expect("connection registry poisoned");
        if self.shutdown.is_cancelled() {
            return Err(DeviceError::Closed);
        }
        self.store.remove(peer)?;
        if let Some((connection, _)) = peers.remove(&peer) {
            connection.close(0_u32.into(), b"peer forgotten");
        }
        Ok(())
    }

    fn get(&self, peer: EndpointId) -> Option<Connection> {
        self.peers
            .lock()
            .expect("connection registry poisoned")
            .get(&peer)
            .map(|(connection, _)| connection)
            .filter(|connection| connection.close_reason().is_none())
            .cloned()
    }

    fn is_connected(&self) -> bool {
        self.peers
            .lock()
            .expect("connection registry poisoned")
            .values()
            .any(|(connection, _)| connection.close_reason().is_none())
    }

    fn close(&self) {
        let mut peers = self.peers.lock().expect("connection registry poisoned");
        for (connection, _) in peers.values() {
            connection.close(0_u32.into(), b"Mist runtime closed");
        }
        peers.clear();
    }
}
