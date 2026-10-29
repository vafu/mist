use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use iroh_tickets::endpoint::EndpointTicket;
use prost::Message;

use super::{Pairing, PairingError, PairingOffer};
use crate::protocol::wire::PairingOffer as WirePairingOffer;
use crate::{DeviceInfo, MistBuilder, Platform, Runtime};

fn device(name: &str) -> DeviceInfo {
    DeviceInfo {
        display_name: name.to_owned(),
        platform: Platform::Linux,
    }
}

#[tokio::test]
async fn pairing_registers_peers_without_replacing_existing_connections() {
    let initiator = MistBuilder::new(device("initiator"))
        .local_only()
        .bind()
        .await
        .unwrap();
    let acceptor = MistBuilder::new(device("acceptor"))
        .local_only()
        .bind()
        .await
        .unwrap();
    let offer = initiator.pairing().initiate().unwrap();

    let waiting = {
        let initiator_pairing = initiator.pairing();
        tokio::spawn(async move { initiator_pairing.wait_for_peer().await })
    };
    let accepted = acceptor.pairing().accept(offer).await.unwrap();
    let invited = tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    assert_eq!(accepted.endpoint_id, initiator.endpoint_id());
    assert_eq!(invited.endpoint_id, acceptor.endpoint_id());
    assert_eq!(accepted.device, device("initiator"));
    assert_eq!(invited.device, device("acceptor"));
    assert_eq!(
        acceptor
            .connection(initiator.address().id)
            .unwrap()
            .remote_id(),
        initiator.address().id
    );
    assert_eq!(
        initiator
            .connection(acceptor.address().id)
            .unwrap()
            .remote_id(),
        acceptor.address().id
    );

    let third = MistBuilder::new(device("third"))
        .local_only()
        .bind()
        .await
        .unwrap();
    let invitation = initiator.pairing();
    let offer = invitation.initiate().unwrap();
    let joining = third.pairing();
    let (invited, accepted) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(invitation.wait_for_peer(), joining.accept(offer))
    })
    .await
    .unwrap();
    assert_eq!(invited.unwrap().endpoint_id, third.endpoint_id());
    assert_eq!(accepted.unwrap().endpoint_id, initiator.endpoint_id());
    assert!(initiator.connection(acceptor.address().id).is_some());
    assert!(initiator.connection(third.address().id).is_some());

    initiator.shutdown().await.unwrap();
    acceptor.shutdown().await.unwrap();
    third.shutdown().await.unwrap();
}

#[tokio::test]
async fn malformed_offer_is_rejected_without_connecting() {
    let runtime = MistBuilder::new(device("runtime"))
        .local_only()
        .bind()
        .await
        .unwrap();
    let error = runtime
        .pairing()
        .accept(PairingOffer {
            encoded: "not a Mist offer".to_owned(),
        })
        .await
        .unwrap_err();

    assert!(matches!(error, PairingError::InvalidOffer { .. }));
    assert!(!runtime.is_connected());
    runtime.shutdown().await.unwrap();
}

#[tokio::test]
async fn invalid_token_is_rejected_without_consuming_the_offer() {
    let initiator = MistBuilder::new(device("initiator"))
        .local_only()
        .bind()
        .await
        .unwrap();
    let unauthorized = MistBuilder::new(device("unauthorized"))
        .local_only()
        .bind()
        .await
        .unwrap();
    let acceptor = MistBuilder::new(device("acceptor"))
        .local_only()
        .bind()
        .await
        .unwrap();
    let offer = initiator.pairing().initiate().unwrap();
    let encoded = offer.encoded.strip_prefix("mist:pair:1:").unwrap();
    let mut wire =
        WirePairingOffer::decode(URL_SAFE_NO_PAD.decode(encoded).unwrap().as_slice()).unwrap();
    wire.pairing_token[0] ^= 0xff;
    let invalid_offer = PairingOffer {
        encoded: format!(
            "mist:pair:1:{}",
            URL_SAFE_NO_PAD.encode(wire.encode_to_vec())
        ),
    };

    let waiting = {
        let pairing = initiator.pairing();
        tokio::spawn(async move { pairing.wait_for_peer().await })
    };
    let error = unauthorized
        .pairing()
        .accept(invalid_offer)
        .await
        .unwrap_err();
    assert!(
        matches!(error, PairingError::Rejected { .. }),
        "unexpected error: {error:?}"
    );
    assert!(!initiator.is_connected());

    let accepted = acceptor.pairing().accept(offer).await.unwrap();
    let invited = tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(accepted.device, device("initiator"));
    assert_eq!(invited.device, device("acceptor"));

    initiator.shutdown().await.unwrap();
    unauthorized.shutdown().await.unwrap();
    acceptor.shutdown().await.unwrap();
}

#[tokio::test]
async fn waiting_requires_an_active_offer() {
    let runtime = MistBuilder::new(device("runtime"))
        .local_only()
        .bind()
        .await
        .unwrap();

    let error = runtime.pairing().wait_for_peer().await.unwrap_err();

    assert!(matches!(error, PairingError::NoPendingOffer));
    runtime.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancelling_the_offer_wakes_the_waiter() {
    let runtime = MistBuilder::new(device("runtime"))
        .local_only()
        .bind()
        .await
        .unwrap();
    let pairing = runtime.pairing();
    pairing.initiate().unwrap();
    let waiting = pairing.wait_for_peer();
    tokio::pin!(waiting);
    std::future::poll_fn(|cx| {
        assert!(std::future::Future::poll(waiting.as_mut(), cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    pairing.cancel();

    let error = waiting.await.unwrap_err();
    assert!(matches!(error, PairingError::Cancelled));
    runtime.shutdown().await.unwrap();
}

#[tokio::test]
async fn stalled_handshake_does_not_block_an_authorized_peer() {
    let initiator = MistBuilder::new(device("initiator"))
        .local_only()
        .bind()
        .await
        .unwrap();
    let stalled_peer = iroh::Endpoint::builder(iroh::endpoint::presets::N0)
        .alpns(vec![crate::MIST_ALPN.to_vec()])
        .clear_address_lookup()
        .relay_mode(iroh::RelayMode::Disabled)
        .bind()
        .await
        .unwrap();
    let acceptor = MistBuilder::new(device("acceptor"))
        .local_only()
        .bind()
        .await
        .unwrap();
    let offer = initiator.pairing().initiate().unwrap();
    let encoded = offer.encoded.strip_prefix("mist:pair:1:").unwrap();
    let wire =
        WirePairingOffer::decode(URL_SAFE_NO_PAD.decode(encoded).unwrap().as_slice()).unwrap();
    let ticket = EndpointTicket::from_str(&wire.endpoint_ticket).unwrap();

    let waiting = {
        let pairing = initiator.pairing();
        tokio::spawn(async move { pairing.wait_for_peer().await })
    };
    let stalled_connection = stalled_peer
        .connect(ticket.endpoint_addr().clone(), crate::MIST_ALPN)
        .await
        .unwrap();

    let accepted = acceptor.pairing().accept(offer).await.unwrap();
    let invited = tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(accepted.device, device("initiator"));
    assert_eq!(invited.device, device("acceptor"));

    stalled_connection.close(0_u32.into(), b"test complete");
    initiator.shutdown().await.unwrap();
    stalled_peer.close().await;
    acceptor.shutdown().await.unwrap();
}

#[test]
fn pairing_is_an_object_safe_generated_contract() {
    fn accepts_trait_object(_: Arc<dyn Pairing>) {}

    let _ = accepts_trait_object;
}

#[tokio::test]
async fn replaced_and_consumed_invitations_cannot_authorize_another_peer() {
    let a = MistBuilder::new(device("a"))
        .local_only()
        .bind()
        .await
        .unwrap();
    let b = MistBuilder::new(device("b"))
        .local_only()
        .bind()
        .await
        .unwrap();
    let c = MistBuilder::new(device("c"))
        .local_only()
        .bind()
        .await
        .unwrap();
    let offering = a.pairing();
    let old = offering.initiate().unwrap();
    let waiting = offering.wait_for_peer();
    tokio::pin!(waiting);
    std::future::poll_fn(|cx| {
        assert!(std::future::Future::poll(waiting.as_mut(), cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    let replacement = offering.initiate().unwrap();
    assert!(matches!(waiting.await, Err(PairingError::Cancelled)));
    assert!(matches!(
        b.pairing().accept(old).await,
        Err(PairingError::Rejected { .. })
    ));
    b.pairing().accept(replacement.clone()).await.unwrap();
    offering.wait_for_peer().await.unwrap();
    assert!(matches!(
        c.pairing().accept(replacement).await,
        Err(PairingError::Rejected { .. })
    ));
    assert_eq!(a.devices().peers().unwrap().len(), 1);
    assert!(c.devices().peers().unwrap().is_empty());
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    c.shutdown().await.unwrap();
}

#[tokio::test]
async fn shutdown_stops_a_handshake_after_token_authorization() {
    use crate::protocol::wire::{ControlFrame, PairingAuthenticate, control_frame};
    use crate::protocol::{read_control_frame, write_control_frame};

    let runtime = MistBuilder::new(device("local"))
        .local_only()
        .bind()
        .await
        .unwrap();
    let offer = runtime.pairing().initiate().unwrap();
    let bytes = URL_SAFE_NO_PAD
        .decode(offer.encoded.strip_prefix("mist:pair:1:").unwrap())
        .unwrap();
    let invitation = WirePairingOffer::decode(bytes.as_slice()).unwrap();
    let endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::N0)
        .clear_address_lookup()
        .relay_mode(iroh::RelayMode::Disabled)
        .bind()
        .await
        .unwrap();
    let connection = endpoint
        .connect(runtime.address(), crate::MIST_ALPN)
        .await
        .unwrap();
    let (mut send, mut recv) = connection.open_bi().await.unwrap();
    write_control_frame(
        &mut send,
        &ControlFrame {
            body: Some(control_frame::Body::PairingAuthenticate(
                PairingAuthenticate {
                    pairing_token: invitation.pairing_token,
                },
            )),
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        read_control_frame(&mut recv).await.unwrap().body,
        Some(control_frame::Body::PairingAuthenticated(_))
    ));
    // The peer deliberately never supplies its metadata; shutdown must cancel
    // the tracked handshake rather than wait indefinitely for network input.
    tokio::time::timeout(Duration::from_secs(5), runtime.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        runtime.pairing().wait_for_peer().await,
        Err(PairingError::RuntimeClosed)
    ));
    connection.closed().await;
    endpoint.close().await;
}
