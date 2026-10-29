use prost::Message;

use super::wire::{
    ControlFrame, DeviceInfo, PairingAuthenticate, PairingOffer, Platform, SessionHello,
    control_frame,
};
use super::{MAX_CONTROL_FRAME_SIZE, ProtocolError, read_control_frame, write_control_frame};

#[test]
fn pairing_offer_round_trips() {
    let offer = PairingOffer {
        format_version: 1,
        endpoint_ticket: "endpoint-ticket".to_owned(),
        pairing_token: vec![42; 32],
    };

    let decoded = PairingOffer::decode(offer.encode_to_vec().as_slice()).unwrap();

    assert_eq!(decoded, offer);
}

#[test]
fn authenticated_session_hello_round_trips_as_a_control_frame() {
    let frame = ControlFrame {
        body: Some(control_frame::Body::SessionHello(SessionHello {
            protocol_version: 1,
            device: Some(DeviceInfo {
                display_name: "bridge".to_owned(),
                platform: Platform::Linux.into(),
            }),
        })),
    };

    let decoded = ControlFrame::decode(frame.encode_to_vec().as_slice()).unwrap();

    assert_eq!(decoded, frame);
}

#[tokio::test]
async fn control_frame_payload_limit_is_symmetric() {
    use std::time::Duration;

    use iroh::{Endpoint, RelayMode, endpoint::presets};
    let endpoint = || {
        Endpoint::builder(presets::N0)
            .alpns(vec![crate::MIST_ALPN.to_vec()])
            .clear_address_lookup()
            .relay_mode(RelayMode::Disabled)
    };
    let first = endpoint().bind().await.unwrap();
    let second = endpoint().bind().await.unwrap();

    tokio::time::timeout(Duration::from_secs(5), async {
        let (outgoing, incoming) =
            tokio::join!(first.connect(second.addr(), crate::MIST_ALPN), async {
                second.accept().await.unwrap().await
            });
        let outgoing = outgoing.unwrap();
        let incoming = incoming.unwrap();
        // Two nested messages each contribute a tag and three-byte length.
        let boundary = ControlFrame {
            body: Some(control_frame::Body::PairingAuthenticate(
                PairingAuthenticate {
                    pairing_token: vec![0; MAX_CONTROL_FRAME_SIZE - 8],
                },
            )),
        };
        assert_eq!(boundary.encoded_len(), MAX_CONTROL_FRAME_SIZE);
        let mut oversized = boundary.clone();
        let Some(control_frame::Body::PairingAuthenticate(authentication)) = &mut oversized.body
        else {
            unreachable!();
        };
        authentication.pairing_token.push(0);
        assert_eq!(oversized.encoded_len(), MAX_CONTROL_FRAME_SIZE + 1);

        let sending = async {
            let mut stream = outgoing.open_uni().await.unwrap();
            assert!(matches!(
                write_control_frame(&mut stream, &oversized).await,
                Err(ProtocolError::FrameTooLarge)
            ));
            write_control_frame(&mut stream, &boundary).await.unwrap();
            // Bypass the writer to verify rejection of an oversized peer frame
            // from its length prefix alone, without sending a payload.
            let mut prefix = Vec::new();
            prost::encode_length_delimiter(MAX_CONTROL_FRAME_SIZE + 1, &mut prefix).unwrap();
            stream.write_all(&prefix).await.unwrap();
            stream.finish().unwrap();
        };
        let receiving = async {
            let mut stream = incoming.accept_uni().await.unwrap();
            assert_eq!(read_control_frame(&mut stream).await.unwrap(), boundary);
            assert!(matches!(
                read_control_frame(&mut stream).await,
                Err(ProtocolError::FrameTooLarge)
            ));
        };
        tokio::join!(sending, receiving);
    })
    .await
    .unwrap();

    first.close().await;
    second.close().await;
}
