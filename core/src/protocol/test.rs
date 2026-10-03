use prost::Message;

use super::wire::{
    Capability, CapabilitySnapshot, CapabilityState, ControlFrame, DeviceClass, DeviceInfo,
    PairingOffer, Platform, SessionHello, control_frame,
};

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
                device_class: DeviceClass::Laptop.into(),
                platform: Platform::Linux.into(),
            }),
            capabilities: Some(CapabilitySnapshot {
                capabilities: vec![Capability {
                    id: "notifications".to_owned(),
                    protocol_version: 1,
                    source_state: CapabilityState::Enabled.into(),
                    sink_state: CapabilityState::Disabled.into(),
                }],
            }),
        })),
    };

    let decoded = ControlFrame::decode(frame.encode_to_vec().as_slice()).unwrap();

    assert_eq!(decoded, frame);
}
