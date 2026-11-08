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

#[test]
fn media_snapshot_and_command_round_trip() {
    use crate::{
        MediaCapabilities, MediaCommand, MediaMetadata, MediaOrigin, MediaPlayer, PlaybackStatus,
    };

    let player = MediaPlayer {
        id: "player-1".to_owned(),
        endpoint_id: String::new(),
        name: "Player".to_owned(),
        device: crate::DeviceInfo {
            display_name: "laptop".to_owned(),
            device_class: crate::DeviceClass::Laptop,
            platform: crate::Platform::Linux,
        },
        origin: MediaOrigin::Local,
        status: PlaybackStatus::Playing,
        metadata: MediaMetadata {
            title: "Track".to_owned(),
            artist: "Artist".to_owned(),
            album: "Album".to_owned(),
            art_url: "https://example.invalid/art.png".to_owned(),
            duration_ms: Some(123_000),
        },
        capabilities: MediaCapabilities {
            commands: vec![MediaCommand::Play, MediaCommand::Pause, MediaCommand::Next],
        },
        position_ms: Some(4_000),
    };

    let snapshot = super::encode_media_snapshot(&[player]);
    let decoded = super::decode_media_snapshot(snapshot).unwrap();
    assert_eq!(decoded.len(), 1);
    let state = &decoded[0];
    assert_eq!(state.id, "player-1");
    assert_eq!(state.status, PlaybackStatus::Playing);
    assert_eq!(state.metadata.title, "Track");
    assert_eq!(state.metadata.duration_ms, Some(123_000));
    assert_eq!(state.position_ms, Some(4_000));
    assert!(state.capabilities.supports(MediaCommand::Next));
    assert!(!state.capabilities.supports(MediaCommand::Previous));

    let wire = super::encode_media_command("player-1", MediaCommand::Previous);
    let (id, command) = super::decode_media_command(wire).unwrap();
    assert_eq!(id, "player-1");
    assert_eq!(command, MediaCommand::Previous);
}

#[test]
fn media_snapshot_rejects_empty_player_ids() {
    use super::wire::{MediaCapabilities, MediaMetadata, MediaSnapshot, MediaState};

    let snapshot = MediaSnapshot {
        players: vec![MediaState {
            player_id: String::new(),
            name: "Player".to_owned(),
            status: 1,
            metadata: Some(MediaMetadata {
                title: String::new(),
                artist: String::new(),
                album: String::new(),
                art_url: String::new(),
                duration_ms: 0,
                has_duration: false,
            }),
            capabilities: Some(MediaCapabilities {
                can_play: true,
                can_pause: true,
                can_go_next: false,
                can_go_previous: false,
            }),
            position_ms: 0,
            has_position: false,
        }],
    };
    assert!(super::decode_media_snapshot(snapshot).is_err());
}
