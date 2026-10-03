use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::{EndpointAddr, EndpointId};
use iroh_tickets::endpoint::EndpointTicket;
use prost::Message;
use subtle::ConstantTimeEq;
use tokio::sync::watch;
use tokio::task::JoinSet;

use super::{PairedPeer, Pairing, PairingError, PairingOffer};
use crate::DeviceInfo;
use crate::protocol::wire::{
    ControlFrame, PairingAuthenticate, PairingAuthenticated, PairingOffer as WirePairingOffer,
    PairingRejected, PairingRejectionReason, control_frame,
};
use crate::protocol::{
    ProtocolError, decode_session_hello, message_name, read_control_frame, session_hello,
    write_control_frame,
};
use crate::runtime::{MistError, Session};

const OFFER_PREFIX: &str = "mist:pair:1:";
const OFFER_FORMAT_VERSION: u32 = 1;
const MAX_ENCODED_OFFER_SIZE: usize = 16 * 1024;
const PAIRING_CLOSE_CODE: u32 = 1;

#[derive(Debug)]
struct ParsedOffer {
    endpoint_addr: EndpointAddr,
    token: [u8; 32],
}

enum IncomingHandshake {
    Attempt {
        connection: Connection,
        send: SendStream,
        recv: RecvStream,
        token: [u8; 32],
    },
    RejectionDelivered,
}

#[derive(Clone, Debug)]
pub(crate) struct IrohPairing {
    session: Arc<Session>,
    pending_offer: watch::Sender<Option<[u8; 32]>>,
}

impl IrohPairing {
    pub(crate) fn new(session: Arc<Session>) -> Self {
        let (pending_offer, _) = watch::channel(None);
        Self {
            session,
            pending_offer,
        }
    }

    fn paired_peer(endpoint_id: EndpointId, device: DeviceInfo) -> PairedPeer {
        PairedPeer {
            endpoint_id: endpoint_id.to_string(),
            device,
        }
    }

    fn parse_offer(offer: &PairingOffer) -> Result<ParsedOffer, PairingError> {
        let encoded =
            offer
                .encoded
                .strip_prefix(OFFER_PREFIX)
                .ok_or_else(|| PairingError::InvalidOffer {
                    reason: "unsupported scheme or protocol version".to_owned(),
                })?;
        if encoded.len() > MAX_ENCODED_OFFER_SIZE {
            return Err(PairingError::InvalidOffer {
                reason: "offer exceeds the maximum encoded size".to_owned(),
            });
        }

        let bytes =
            URL_SAFE_NO_PAD
                .decode(encoded)
                .map_err(|error| PairingError::InvalidOffer {
                    reason: format!("invalid base64 payload: {error}"),
                })?;
        let wire = WirePairingOffer::decode(bytes.as_slice()).map_err(|error| {
            PairingError::InvalidOffer {
                reason: format!("invalid protobuf payload: {error}"),
            }
        })?;
        if wire.format_version != OFFER_FORMAT_VERSION {
            return Err(PairingError::InvalidOffer {
                reason: format!("unsupported offer format version {}", wire.format_version),
            });
        }
        let token =
            wire.pairing_token
                .as_slice()
                .try_into()
                .map_err(|_| PairingError::InvalidOffer {
                    reason: "pairing token must contain exactly 32 bytes".to_owned(),
                })?;
        let ticket = EndpointTicket::from_str(&wire.endpoint_ticket).map_err(|error| {
            PairingError::InvalidOffer {
                reason: format!("invalid Iroh endpoint ticket: {error}"),
            }
        })?;

        Ok(ParsedOffer {
            endpoint_addr: ticket.endpoint_addr().clone(),
            token,
        })
    }

    fn active_token(&self) -> Result<[u8; 32], PairingError> {
        (*self.pending_offer.borrow()).ok_or(PairingError::NoPendingOffer)
    }

    fn consume_pending_offer(&self, token: &[u8; 32]) -> Result<(), PairingError> {
        let mut consumed = false;
        self.pending_offer.send_if_modified(|pending| {
            if pending
                .as_ref()
                .is_some_and(|active| bool::from(active.ct_eq(token)))
            {
                *pending = None;
                consumed = true;
                true
            } else {
                false
            }
        });
        if consumed {
            Ok(())
        } else {
            Err(PairingError::Rejected {
                reason: "the invitation is no longer active".to_owned(),
            })
        }
    }

    fn protocol(error: impl std::fmt::Display) -> PairingError {
        PairingError::Protocol {
            reason: error.to_string(),
        }
    }

    fn connect_error(error: MistError) -> PairingError {
        match error {
            MistError::ConnectedToDifferentPeer => PairingError::AlreadyPaired,
            other => PairingError::Connect {
                reason: other.to_string(),
            },
        }
    }

    fn accept_error(error: MistError) -> PairingError {
        match error {
            MistError::ConnectedToDifferentPeer => PairingError::AlreadyPaired,
            MistError::EndpointClosed => PairingError::RuntimeClosed,
            MistError::Accept(error) => PairingError::Accept {
                reason: error.to_string(),
            },
            other => PairingError::Accept {
                reason: other.to_string(),
            },
        }
    }

    async fn exchange_device_info(
        &self,
        send: &mut SendStream,
        recv: &mut RecvStream,
    ) -> Result<DeviceInfo, PairingError> {
        let hello = ControlFrame {
            body: Some(control_frame::Body::SessionHello(session_hello(
                self.session.device_info(),
            ))),
        };
        write_control_frame(send, &hello)
            .await
            .map_err(Self::protocol)?;

        let response = read_control_frame(recv).await.map_err(Self::protocol)?;
        let body = response
            .body
            .ok_or_else(|| Self::protocol(ProtocolError::EmptyFrame))?;
        match body {
            control_frame::Body::SessionHello(hello) => {
                decode_session_hello(hello).map_err(Self::protocol)
            }
            other => Err(Self::protocol(ProtocolError::UnexpectedMessage {
                expected: "session hello",
                received: message_name(&other),
            })),
        }
    }

    async fn authenticate_with_creator(
        &self,
        connection: &Connection,
        token: [u8; 32],
    ) -> Result<DeviceInfo, PairingError> {
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
        let body = response
            .body
            .ok_or_else(|| Self::protocol(ProtocolError::EmptyFrame))?;
        match body {
            control_frame::Body::PairingAuthenticated(_) => {}
            control_frame::Body::PairingRejected(rejection) => {
                return Err(PairingError::Rejected {
                    reason: rejection_reason(rejection.reason).to_owned(),
                });
            }
            other => {
                return Err(Self::protocol(ProtocolError::UnexpectedMessage {
                    expected: "pairing acknowledgement",
                    received: message_name(&other),
                }));
            }
        }

        let device = self.exchange_device_info(&mut send, &mut recv).await?;
        send.finish().map_err(Self::protocol)?;
        Ok(device)
    }

    async fn receive_authentication(
        connection: Connection,
    ) -> Result<IncomingHandshake, PairingError> {
        let (send, mut recv) = connection.accept_bi().await.map_err(Self::protocol)?;
        let request = read_control_frame(&mut recv)
            .await
            .map_err(Self::protocol)?;
        let body = request
            .body
            .ok_or_else(|| Self::protocol(ProtocolError::EmptyFrame))?;
        let control_frame::Body::PairingAuthenticate(authentication) = body else {
            return Err(Self::protocol(ProtocolError::UnexpectedMessage {
                expected: "pairing authentication",
                received: message_name(&body),
            }));
        };
        let token = authentication
            .pairing_token
            .as_slice()
            .try_into()
            .map_err(|_| {
                Self::protocol("pairing authentication token must contain exactly 32 bytes")
            })?;
        Ok(IncomingHandshake::Attempt {
            connection,
            send,
            recv,
            token,
        })
    }

    async fn deliver_rejection(
        connection: Connection,
        mut send: SendStream,
        reason: PairingRejectionReason,
    ) -> Result<IncomingHandshake, PairingError> {
        write_control_frame(
            &mut send,
            &ControlFrame {
                body: Some(control_frame::Body::PairingRejected(PairingRejected {
                    reason: reason.into(),
                })),
            },
        )
        .await
        .map_err(Self::protocol)?;
        send.finish().map_err(Self::protocol)?;
        let _ = send.stopped().await;
        connection.close(PAIRING_CLOSE_CODE.into(), b"pairing rejected");
        Ok(IncomingHandshake::RejectionDelivered)
    }
}

#[async_trait]
impl Pairing for IrohPairing {
    fn initiate(&self) -> Result<PairingOffer, PairingError> {
        if self.session.connection().is_some() {
            return Err(PairingError::AlreadyPaired);
        }

        let mut token = [0_u8; 32];
        getrandom::fill(&mut token).map_err(|error| PairingError::TokenGeneration {
            reason: error.to_string(),
        })?;
        let payload = WirePairingOffer {
            format_version: OFFER_FORMAT_VERSION,
            endpoint_ticket: EndpointTicket::new(self.session.endpoint().addr()).to_string(),
            pairing_token: token.to_vec(),
        };
        let encoded = format!(
            "{OFFER_PREFIX}{}",
            URL_SAFE_NO_PAD.encode(payload.encode_to_vec())
        );

        self.pending_offer.send_replace(Some(token));
        Ok(PairingOffer { encoded })
    }

    fn cancel(&self) {
        self.pending_offer.send_replace(None);
    }

    async fn accept(&self, offer: PairingOffer) -> Result<PairedPeer, PairingError> {
        let pairing = self.clone();
        let executor = self.session.executor().clone();
        executor
            .spawn(async move {
                if pairing.session.connection().is_some() {
                    return Err(PairingError::AlreadyPaired);
                }
                let offer = Self::parse_offer(&offer)?;
                let connection = pairing
                    .session
                    .connect_candidate(offer.endpoint_addr)
                    .await
                    .map_err(Self::connect_error)?;

                let device = pairing
                    .authenticate_with_creator(&connection, offer.token)
                    .await?;
                let remote_id = connection.remote_id();
                pairing
                    .session
                    .install(connection)
                    .map_err(Self::connect_error)?;
                Ok(Self::paired_peer(remote_id, device))
            })
            .await
            .map_err(|error| PairingError::Executor {
                reason: error.to_string(),
            })?
    }

    async fn wait_for_peer(&self) -> Result<PairedPeer, PairingError> {
        let pairing = self.clone();
        let executor = self.session.executor().clone();
        executor
            .spawn(async move {
                if pairing.session.connection().is_some() {
                    return Err(PairingError::AlreadyPaired);
                }
                pairing.active_token()?;

                let mut offers = pairing.pending_offer.subscribe();
                let mut handshakes = JoinSet::new();
                loop {
                    tokio::select! {
                        changed = offers.changed() => {
                            if changed.is_err() || offers.borrow().is_none() {
                                return Err(PairingError::Cancelled);
                            }
                        }
                        candidate = pairing.session.accept_candidate() => {
                            let connection = candidate.map_err(Self::accept_error)?;
                            handshakes.spawn(Self::receive_authentication(connection));
                        }
                        completed = handshakes.join_next(), if !handshakes.is_empty() => {
                            let Some(Ok(Ok(handshake))) = completed else {
                                continue;
                            };
                            let IncomingHandshake::Attempt {
                                connection,
                                mut send,
                                mut recv,
                                token,
                            } = handshake else {
                                continue;
                            };

                            let Some(active_token) = *offers.borrow() else {
                                handshakes.spawn(Self::deliver_rejection(
                                    connection,
                                    send,
                                    PairingRejectionReason::InvalidToken,
                                ));
                                continue;
                            };
                            if !bool::from(active_token.ct_eq(&token)) {
                                handshakes.spawn(Self::deliver_rejection(
                                    connection,
                                    send,
                                    PairingRejectionReason::InvalidToken,
                                ));
                                continue;
                            }
                            if pairing.consume_pending_offer(&token).is_err() {
                                handshakes.spawn(Self::deliver_rejection(
                                    connection,
                                    send,
                                    PairingRejectionReason::InvalidToken,
                                ));
                                continue;
                            }

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
                            let device = pairing.exchange_device_info(&mut send, &mut recv).await?;
                            send.finish().map_err(Self::protocol)?;

                            let remote_id = connection.remote_id();
                            pairing
                                .session
                                .install(connection)
                                .map_err(Self::connect_error)?;
                            return Ok(Self::paired_peer(remote_id, device));
                        }
                    }
                }
            })
            .await
            .map_err(|error| PairingError::Executor {
                reason: error.to_string(),
            })?
    }
}

fn rejection_reason(reason: i32) -> &'static str {
    match PairingRejectionReason::try_from(reason) {
        Ok(PairingRejectionReason::InvalidToken) => "invalid pairing token",
        Ok(PairingRejectionReason::AlreadyPaired) => "device is already paired",
        Ok(PairingRejectionReason::Incompatible) => "incompatible Mist protocol",
        Ok(PairingRejectionReason::Unspecified) | Err(_) => "unspecified rejection",
    }
}
