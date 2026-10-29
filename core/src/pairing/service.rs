use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::{EndpointAddr, EndpointId};
use iroh_tickets::endpoint::EndpointTicket;
use prost::Message;
use subtle::ConstantTimeEq;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::{IncomingPairing, Pairing, PairingError, PairingOffer};
use crate::protocol::wire::{
    ControlFrame, PairingAuthenticate, PairingAuthenticated, PairingOffer as WirePairingOffer,
    PairingRejected, PairingRejectionReason, control_frame,
};
use crate::protocol::{
    decode_session_hello, read_control_frame, session_hello, write_control_frame,
};
use crate::runtime::{Connections, RuntimeError, run_owned};
use crate::{DeviceInfo, Peer};

const OFFER_PREFIX: &str = "mist:pair:1:";
const MAX_ENCODED_OFFER_SIZE: usize = 16 * 1024;

#[derive(Debug)]
struct Invitation {
    token: [u8; 32],
    consumed: AtomicBool,
    cancelled: CancellationToken,
    result: watch::Sender<Option<Result<Peer, PairingError>>>,
}

#[derive(Clone, Debug)]
pub(crate) struct IrohPairing {
    connections: Arc<dyn Connections>,
    local_device: DeviceInfo,
    pending: Arc<Mutex<Option<Arc<Invitation>>>>,
    shutdown: CancellationToken,
    tasks: TaskTracker,
}

impl IrohPairing {
    pub(crate) fn new(
        connections: Arc<dyn Connections>,
        local_device: DeviceInfo,
        shutdown: CancellationToken,
        tasks: TaskTracker,
    ) -> Self {
        Self {
            connections,
            local_device,
            pending: Arc::new(Mutex::new(None)),
            shutdown,
            tasks,
        }
    }

    fn peer(endpoint_id: EndpointId, device: DeviceInfo) -> Peer {
        Peer {
            endpoint_id: endpoint_id.to_string(),
            device,
        }
    }

    fn parse_offer(offer: &PairingOffer) -> Result<(EndpointAddr, [u8; 32]), PairingError> {
        let invalid = |reason: &str| PairingError::InvalidOffer {
            reason: reason.to_owned(),
        };
        let encoded = offer
            .encoded
            .strip_prefix(OFFER_PREFIX)
            .ok_or_else(|| invalid("unsupported scheme or protocol version"))?;
        if encoded.len() > MAX_ENCODED_OFFER_SIZE {
            return Err(invalid("offer exceeds the maximum encoded size"));
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|error| invalid(&error.to_string()))?;
        let wire = WirePairingOffer::decode(bytes.as_slice())
            .map_err(|error| invalid(&error.to_string()))?;
        if wire.format_version != 1 {
            return Err(invalid("unsupported offer version"));
        }
        let token = wire
            .pairing_token
            .as_slice()
            .try_into()
            .map_err(|_| invalid("pairing token must contain exactly 32 bytes"))?;
        let ticket = EndpointTicket::from_str(&wire.endpoint_ticket)
            .map_err(|error| invalid(&error.to_string()))?;
        Ok((ticket.endpoint_addr().clone(), token))
    }

    fn protocol(error: impl std::fmt::Display) -> PairingError {
        PairingError::Protocol {
            reason: error.to_string(),
        }
    }

    fn runtime(error: RuntimeError) -> PairingError {
        match error {
            RuntimeError::Closed => PairingError::RuntimeClosed,
            RuntimeError::Storage { reason } => PairingError::Storage { reason },
            other => PairingError::Executor {
                reason: other.to_string(),
            },
        }
    }

    async fn exchange(
        &self,
        send: &mut SendStream,
        recv: &mut RecvStream,
    ) -> Result<DeviceInfo, PairingError> {
        write_control_frame(
            send,
            &ControlFrame {
                body: Some(control_frame::Body::SessionHello(session_hello(
                    &self.local_device,
                ))),
            },
        )
        .await
        .map_err(Self::protocol)?;
        let response = read_control_frame(recv).await.map_err(Self::protocol)?;
        match response.body {
            Some(control_frame::Body::SessionHello(hello)) => {
                decode_session_hello(&hello).map_err(Self::protocol)
            }
            _ => Err(Self::protocol("expected session hello")),
        }
    }

    async fn accept_offer(&self, offer: PairingOffer) -> Result<Peer, PairingError> {
        let (address, token) = Self::parse_offer(&offer)?;
        let connection =
            self.connections
                .dial(address)
                .await
                .map_err(|error| PairingError::Connect {
                    reason: error.to_string(),
                })?;
        let result = async {
            let (mut send, mut recv) = connection.open_bi().await.map_err(Self::protocol)?;
            write_control_frame(
                &mut send,
                &ControlFrame {
                    body: Some(control_frame::Body::PairingAuthenticate(
                        PairingAuthenticate {
                            pairing_token: token.to_vec(),
                        },
                    )),
                },
            )
            .await
            .map_err(Self::protocol)?;
            let response = read_control_frame(&mut recv)
                .await
                .map_err(Self::protocol)?;
            match response.body {
                Some(control_frame::Body::PairingAuthenticated(_)) => {}
                Some(control_frame::Body::PairingRejected(_)) => {
                    return Err(PairingError::Rejected {
                        reason: "invalid or consumed invitation".to_owned(),
                    });
                }
                _ => return Err(Self::protocol("expected pairing acknowledgement")),
            }
            let device = self.exchange(&mut send, &mut recv).await?;
            let peer = Self::peer(connection.remote_id(), device);
            let registered = self
                .connections
                .trust(peer.clone(), connection.clone(), &self.shutdown)
                .map_err(Self::runtime)?;
            send.finish().map_err(Self::protocol)?;
            recv.read_to_end(0).await.map_err(Self::protocol)?;
            if registered.stable_id() != connection.stable_id() {
                connection.close(2_u32.into(), b"pairing used existing connection");
            }
            Ok(peer)
        }
        .await;
        if result.is_err() {
            connection.close(1_u32.into(), b"pairing failed");
        }
        result
    }
}

#[async_trait]
impl Pairing for IrohPairing {
    fn initiate(&self) -> Result<PairingOffer, PairingError> {
        if self.shutdown.is_cancelled() {
            return Err(PairingError::RuntimeClosed);
        }
        let mut token = [0; 32];
        getrandom::fill(&mut token).map_err(|error| PairingError::TokenGeneration {
            reason: error.to_string(),
        })?;
        let offer = WirePairingOffer {
            format_version: 1,
            endpoint_ticket: EndpointTicket::new(self.connections.address()).to_string(),
            pairing_token: token.to_vec(),
        };
        let invitation = Arc::new(Invitation {
            token,
            consumed: AtomicBool::new(false),
            cancelled: CancellationToken::new(),
            result: watch::channel(None).0,
        });
        let mut pending = self.pending.lock().expect("invitation state poisoned");
        if self.shutdown.is_cancelled() {
            return Err(PairingError::RuntimeClosed);
        }
        if let Some(previous) = pending.replace(invitation) {
            previous.cancelled.cancel();
        }
        Ok(PairingOffer {
            encoded: format!(
                "{OFFER_PREFIX}{}",
                URL_SAFE_NO_PAD.encode(offer.encode_to_vec())
            ),
        })
    }

    fn cancel(&self) {
        if let Some(invitation) = self
            .pending
            .lock()
            .expect("invitation state poisoned")
            .take()
        {
            invitation.cancelled.cancel();
        }
    }

    async fn accept(&self, offer: PairingOffer) -> Result<Peer, PairingError> {
        let pairing = self.clone();
        run_owned(&self.tasks, &self.shutdown, async move {
            Ok(pairing.accept_offer(offer).await)
        })
        .await
        .map_err(Self::runtime)?
    }

    async fn wait_for_peer(&self) -> Result<Peer, PairingError> {
        if self.shutdown.is_cancelled() {
            return Err(PairingError::RuntimeClosed);
        }
        let invitation = self
            .pending
            .lock()
            .expect("invitation state poisoned")
            .clone()
            .ok_or(PairingError::NoPendingOffer)?;
        let mut result = invitation.result.subscribe();
        run_owned(&self.tasks, &self.shutdown, async move {
            loop {
                if let Some(result) = result.borrow().clone() { return Ok(result); }
                tokio::select! {
                    biased;
                    () = invitation.cancelled.cancelled() => return Ok(Err(PairingError::Cancelled)),
                    changed = result.changed() => {
                        if changed.is_err() { return Ok(Err(PairingError::Cancelled)); }
                    }
                }
            }
        }).await.map_err(Self::runtime)?
    }
}

#[async_trait]
impl IncomingPairing for IrohPairing {
    async fn authorize(
        &self,
        connection: Connection,
        mut send: SendStream,
        mut recv: RecvStream,
        token: Vec<u8>,
    ) -> Result<(), PairingError> {
        let invitation = self
            .pending
            .lock()
            .expect("invitation state poisoned")
            .clone();
        let invitation = invitation.filter(|invitation| {
            !invitation.cancelled.is_cancelled()
                && bool::from(invitation.token.as_slice().ct_eq(&token))
                && invitation
                    .consumed
                    .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
        });
        let Some(invitation) = invitation else {
            write_control_frame(
                &mut send,
                &ControlFrame {
                    body: Some(control_frame::Body::PairingRejected(PairingRejected {
                        reason: PairingRejectionReason::InvalidToken.into(),
                    })),
                },
            )
            .await
            .map_err(Self::protocol)?;
            send.finish().map_err(Self::protocol)?;
            let _ = send.stopped().await;
            return Err(PairingError::Rejected {
                reason: "invalid pairing token".to_owned(),
            });
        };
        let handshake = async {
            write_control_frame(
                &mut send,
                &ControlFrame {
                    body: Some(control_frame::Body::PairingAuthenticated(
                        PairingAuthenticated {},
                    )),
                },
            )
            .await
            .map_err(Self::protocol)?;
            let device = self.exchange(&mut send, &mut recv).await?;
            if invitation.cancelled.is_cancelled() {
                return Err(PairingError::Cancelled);
            }
            self.connections
                .trust(
                    Self::peer(connection.remote_id(), device.clone()),
                    connection.clone(),
                    &invitation.cancelled,
                )
                .map_err(Self::runtime)?;
            send.finish().map_err(Self::protocol)?;
            Ok(Self::peer(connection.remote_id(), device))
        };
        let result = tokio::select! {
            biased;
            () = invitation.cancelled.cancelled() => Err(PairingError::Cancelled),
            result = handshake => result,
        };
        invitation.result.send_replace(Some(result.clone()));
        result.map(|_| ())
    }
}
