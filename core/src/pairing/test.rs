use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use iroh_tickets::endpoint::EndpointTicket;
use prost::Message;

use super::{Pairing, PairingError, PairingOffer};
use crate::protocol::wire::PairingOffer as WirePairingOffer;
use crate::{DeviceClass, DeviceInfo, MIST_ALPN, MistBuilder, Platform};

fn device(name: &str) -> DeviceInfo {
    DeviceInfo {
        display_name: name.to_owned(),
        device_class: DeviceClass::Desktop,
        platform: Platform::Linux,
    }
}

#[tokio::test]
async fn offer_establishes_the_runtime_connection() {
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

    assert_eq!(accepted.endpoint_id, initiator.endpoint().id().to_string());
    assert_eq!(invited.endpoint_id, acceptor.endpoint().id().to_string());
    assert_eq!(accepted.device, device("initiator"));
    assert_eq!(invited.device, device("acceptor"));
    assert_eq!(
        acceptor.connection().unwrap().remote_id(),
        initiator.endpoint().id()
    );
    assert_eq!(
        initiator.connection().unwrap().remote_id(),
        acceptor.endpoint().id()
    );

    initiator.shutdown().await;
    acceptor.shutdown().await;
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
    assert!(runtime.connection().is_none());
    runtime.shutdown().await;
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
    assert!(initiator.connection().is_none());

    let accepted = acceptor.pairing().accept(offer).await.unwrap();
    let invited = tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(accepted.device, device("initiator"));
    assert_eq!(invited.device, device("acceptor"));

    initiator.shutdown().await;
    unauthorized.shutdown().await;
    acceptor.shutdown().await;
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
    runtime.shutdown().await;
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
    let waiting = {
        let pairing = Arc::clone(&pairing);
        tokio::spawn(async move { pairing.wait_for_peer().await })
    };

    tokio::task::yield_now().await;
    pairing.cancel();

    let error = waiting.await.unwrap().unwrap_err();
    assert!(matches!(error, PairingError::Cancelled));
    runtime.shutdown().await;
}

#[tokio::test]
async fn stalled_handshake_does_not_block_an_authorized_peer() {
    let initiator = MistBuilder::new(device("initiator"))
        .local_only()
        .bind()
        .await
        .unwrap();
    let stalled_peer = MistBuilder::new(device("stalled"))
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
    let wire =
        WirePairingOffer::decode(URL_SAFE_NO_PAD.decode(encoded).unwrap().as_slice()).unwrap();
    let ticket = EndpointTicket::from_str(&wire.endpoint_ticket).unwrap();

    let waiting = {
        let pairing = initiator.pairing();
        tokio::spawn(async move { pairing.wait_for_peer().await })
    };
    let stalled_connection = stalled_peer
        .endpoint()
        .connect(ticket.endpoint_addr().clone(), MIST_ALPN)
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
    initiator.shutdown().await;
    stalled_peer.shutdown().await;
    acceptor.shutdown().await;
}

#[test]
fn pairing_is_an_object_safe_generated_contract() {
    fn accepts_trait_object(_: Arc<dyn Pairing>) {}

    let _ = accepts_trait_object;
}
