use prost::Message;

use crate::{Notification, NotificationKind};

use super::wire::{
    Capability, CapabilitySnapshot, CapabilityState, ControlFrame, DeviceClass, DeviceInfo,
    PairingOffer, Platform, SessionHello, control_frame,
};
use super::{decode_notification, encode_notification};

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
            addresses: Some(super::wire::AddressSnapshot {
                revision: 1,
                addresses: Vec::new(),
            }),
        })),
    };

    let decoded = ControlFrame::decode(frame.encode_to_vec().as_slice()).unwrap();

    assert_eq!(decoded, frame);
}

#[test]
fn notification_round_trips_between_domain_and_wire_types() {
    let notification = Notification {
        id: "mail:42".to_owned(),
        application: "Mail".to_owned(),
        title: "New message".to_owned(),
        body: "The bridge is ready".to_owned(),
        timestamp_ms: 1_791_015_507_000,
        silent: true,
        kind: NotificationKind::Message,
    };

    let frame = ControlFrame {
        body: Some(control_frame::Body::Notification(encode_notification(
            &notification,
        ))),
    };
    let decoded = ControlFrame::decode(frame.encode_to_vec().as_slice()).unwrap();
    let control_frame::Body::Notification(wire_notification) = decoded.body.unwrap() else {
        panic!("notification frame decoded as another control message");
    };

    assert_eq!(
        decode_notification(wire_notification).unwrap(),
        notification
    );
}
