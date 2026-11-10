//! Private generated types for Mist's versioned wire protocol.
//!
//! These types must not cross the crate's public or `UniFFI` boundaries. Public
//! domain types are converted to and from this representation explicitly.

use std::net::SocketAddr;
use std::str::FromStr;

use iroh::endpoint::{RecvStream, SendStream};
use iroh::{EndpointAddr, RelayUrl, TransportAddr};
use iroh_base::CustomAddr;
use iroh_blobs::ticket::BlobTicket;
use prost::Message;
use thiserror::Error;

use crate::{DeviceClass, DeviceInfo, Notification, NotificationKind, Platform};

pub(crate) use self::wire::clipboard_item::Payload as ClipboardPayload;

pub(crate) const SESSION_PROTOCOL_VERSION: u32 = 1;
const MAX_CONTROL_FRAME_SIZE: usize = 64 * 1024;

#[allow(
    dead_code,
    clippy::doc_markdown,
    clippy::trivially_copy_pass_by_ref,
    clippy::struct_excessive_bools,
    reason = "wire messages are landed before their protocol handlers"
)]
pub(crate) mod wire {
    include!(concat!(env!("OUT_DIR"), "/mist.v1.rs"));
}

#[derive(Debug, Error)]
pub(crate) enum ProtocolError {
    #[error("control frame exceeds the {MAX_CONTROL_FRAME_SIZE}-byte limit")]
    FrameTooLarge,
    #[error("invalid control frame length: {0}")]
    InvalidLength(String),
    #[error("failed to decode control frame: {0}")]
    Decode(#[source] prost::DecodeError),
    #[error("failed to read control frame: {0}")]
    Read(String),
    #[error("failed to write control frame: {0}")]
    Write(String),
    #[error("control frame did not contain a message")]
    EmptyFrame,
    #[error("expected {expected}, received {received}")]
    UnexpectedMessage {
        expected: &'static str,
        received: &'static str,
    },
    #[error("session protocol version {0} is not supported")]
    UnsupportedSessionVersion(u32),
    #[error("session hello did not contain device information")]
    MissingDeviceInfo,
    #[error("session hello did not contain a capability snapshot")]
    MissingCapabilities,
    #[error("device display name must contain between 1 and 128 characters")]
    InvalidDisplayName,
    #[error("notification field {0} exceeds its protocol limit")]
    InvalidNotification(&'static str),
    #[error("notification image is too large")]
    NotificationImageTooLarge,
    #[error("session hello did not contain an address snapshot")]
    MissingAddresses,
    #[error("invalid transport address: {0}")]
    InvalidAddress(String),
    #[error("clipboard field {0} exceeds its protocol limit")]
    InvalidClipboard(&'static str),
    #[error("clipboard payload does not match its MIME type")]
    ClipboardPayloadMismatch,
    #[error("media field {0} exceeds its protocol limit")]
    InvalidMedia(&'static str),
    #[error("file offer field {0} exceeds its protocol limit")]
    InvalidFileOffer(&'static str),
}

pub(crate) async fn write_control_frame(
    stream: &mut SendStream,
    frame: &wire::ControlFrame,
) -> Result<(), ProtocolError> {
    let encoded = frame.encode_length_delimited_to_vec();
    if encoded.len() > MAX_CONTROL_FRAME_SIZE {
        return Err(ProtocolError::FrameTooLarge);
    }
    stream
        .write_all(&encoded)
        .await
        .map_err(|error| ProtocolError::Write(error.to_string()))
}

pub(crate) async fn read_control_frame(
    stream: &mut RecvStream,
) -> Result<wire::ControlFrame, ProtocolError> {
    let mut prefix = Vec::with_capacity(10);
    for _ in 0..10 {
        let mut byte = [0_u8; 1];
        stream
            .read_exact(&mut byte)
            .await
            .map_err(|error| ProtocolError::Read(error.to_string()))?;
        prefix.push(byte[0]);
        if byte[0] & 0x80 == 0 {
            break;
        }
    }
    if prefix.last().is_none_or(|byte| byte & 0x80 != 0) {
        return Err(ProtocolError::InvalidLength(
            "length delimiter exceeds ten bytes".to_owned(),
        ));
    }

    let length = prost::decode_length_delimiter(prefix.as_slice())
        .map_err(|error| ProtocolError::InvalidLength(error.to_string()))?;
    if length > MAX_CONTROL_FRAME_SIZE {
        return Err(ProtocolError::FrameTooLarge);
    }

    let mut encoded = vec![0_u8; length];
    stream
        .read_exact(&mut encoded)
        .await
        .map_err(|error| ProtocolError::Read(error.to_string()))?;
    wire::ControlFrame::decode(encoded.as_slice()).map_err(ProtocolError::Decode)
}

#[derive(Debug)]
pub(crate) struct SessionHello {
    pub(crate) device: DeviceInfo,
    pub(crate) addresses: Vec<TransportAddr>,
}

pub(crate) fn session_hello(device: &DeviceInfo, endpoint: EndpointAddr) -> wire::SessionHello {
    wire::SessionHello {
        protocol_version: SESSION_PROTOCOL_VERSION,
        device: Some(encode_device(device)),
        capabilities: Some(wire::CapabilitySnapshot {
            capabilities: Vec::new(),
        }),
        addresses: Some(address_snapshot(0, endpoint.addrs)),
    }
}

pub(crate) fn decode_session_hello(
    hello: wire::SessionHello,
) -> Result<SessionHello, ProtocolError> {
    if hello.protocol_version != SESSION_PROTOCOL_VERSION {
        return Err(ProtocolError::UnsupportedSessionVersion(
            hello.protocol_version,
        ));
    }
    let _capabilities = hello
        .capabilities
        .ok_or(ProtocolError::MissingCapabilities)?;
    let device = decode_device(
        hello
            .device
            .as_ref()
            .ok_or(ProtocolError::MissingDeviceInfo)?,
    )?;
    let addresses =
        decode_address_snapshot(hello.addresses.ok_or(ProtocolError::MissingAddresses)?)?;
    Ok(SessionHello { device, addresses })
}

pub(crate) fn address_snapshot(
    revision: u64,
    addresses: impl IntoIterator<Item = TransportAddr>,
) -> wire::AddressSnapshot {
    wire::AddressSnapshot {
        revision,
        addresses: addresses
            .into_iter()
            .map(|address| wire::TransportAddress {
                value: Some(match address {
                    TransportAddr::Relay(value) => {
                        wire::transport_address::Value::RelayUrl(value.to_string())
                    }
                    TransportAddr::Ip(value) => {
                        wire::transport_address::Value::IpSocket(value.to_string())
                    }
                    TransportAddr::Custom(value) => {
                        wire::transport_address::Value::Custom(value.to_string())
                    }
                    _ => return wire::TransportAddress { value: None },
                }),
            })
            .filter(|address| address.value.is_some())
            .collect(),
    }
}

pub(crate) fn decode_address_snapshot(
    snapshot: wire::AddressSnapshot,
) -> Result<Vec<TransportAddr>, ProtocolError> {
    snapshot
        .addresses
        .into_iter()
        .map(|address| match address.value {
            Some(wire::transport_address::Value::RelayUrl(value)) => RelayUrl::from_str(&value)
                .map(TransportAddr::Relay)
                .map_err(|error| ProtocolError::InvalidAddress(error.to_string())),
            Some(wire::transport_address::Value::IpSocket(value)) => SocketAddr::from_str(&value)
                .map(TransportAddr::Ip)
                .map_err(|error| ProtocolError::InvalidAddress(error.to_string())),
            Some(wire::transport_address::Value::Custom(value)) => CustomAddr::from_str(&value)
                .map(TransportAddr::Custom)
                .map_err(|error| ProtocolError::InvalidAddress(error.to_string())),
            None => Err(ProtocolError::InvalidAddress(
                "transport address has no value".to_owned(),
            )),
        })
        .collect()
}

fn encode_device(device: &DeviceInfo) -> wire::DeviceInfo {
    wire::DeviceInfo {
        display_name: device.display_name.clone(),
        device_class: match device.device_class {
            DeviceClass::Phone => wire::DeviceClass::Phone,
            DeviceClass::Tablet => wire::DeviceClass::Tablet,
            DeviceClass::Laptop => wire::DeviceClass::Laptop,
            DeviceClass::Desktop => wire::DeviceClass::Desktop,
            DeviceClass::Server => wire::DeviceClass::Server,
            DeviceClass::Other => wire::DeviceClass::Other,
        }
        .into(),
        platform: match device.platform {
            Platform::Android => wire::Platform::Android,
            Platform::Linux => wire::Platform::Linux,
            Platform::Ios => wire::Platform::Ios,
            Platform::Macos => wire::Platform::Macos,
            Platform::Windows => wire::Platform::Windows,
            Platform::Other => wire::Platform::Other,
        }
        .into(),
    }
}

fn decode_device(device: &wire::DeviceInfo) -> Result<DeviceInfo, ProtocolError> {
    let display_name = device.display_name.trim().to_owned();
    if display_name.is_empty() || display_name.chars().count() > 128 {
        return Err(ProtocolError::InvalidDisplayName);
    }

    Ok(DeviceInfo {
        display_name,
        device_class: match wire::DeviceClass::try_from(device.device_class) {
            Ok(wire::DeviceClass::Phone) => DeviceClass::Phone,
            Ok(wire::DeviceClass::Tablet) => DeviceClass::Tablet,
            Ok(wire::DeviceClass::Laptop) => DeviceClass::Laptop,
            Ok(wire::DeviceClass::Desktop) => DeviceClass::Desktop,
            Ok(wire::DeviceClass::Server) => DeviceClass::Server,
            Ok(wire::DeviceClass::Unspecified | wire::DeviceClass::Other) | Err(_) => {
                DeviceClass::Other
            }
        },
        platform: match wire::Platform::try_from(device.platform) {
            Ok(wire::Platform::Android) => Platform::Android,
            Ok(wire::Platform::Linux) => Platform::Linux,
            Ok(wire::Platform::Ios) => Platform::Ios,
            Ok(wire::Platform::Macos) => Platform::Macos,
            Ok(wire::Platform::Windows) => Platform::Windows,
            Ok(wire::Platform::Unspecified | wire::Platform::Other) | Err(_) => Platform::Other,
        },
    })
}

/// Decoded notification plus any images that follow on the stream.
#[derive(Debug)]
pub(crate) struct DecodedNotification {
    pub(crate) notification: Notification,
    /// Image tickets in stream order: app icon, then content image.
    pub(crate) images: Vec<BlobTicket>,
}

pub(crate) const MAX_NOTIFICATION_ACTIONS: usize = 8;
pub(crate) const MAX_NOTIFICATION_ACTION_ID: usize = 128;
pub(crate) const MAX_NOTIFICATION_ACTION_LABEL: usize = 128;
pub(crate) const MAX_NOTIFICATION_ICON_BYTES: u64 = 512 * 1024;
pub(crate) const MAX_NOTIFICATION_IMAGE_BYTES: u64 = 8 * 1024 * 1024;

pub(crate) fn wire_image(
    mime_type: &str,
    size: u64,
    hash: iroh_blobs::Hash,
    ticket: &str,
) -> wire::NotificationImage {
    wire::NotificationImage {
        mime_type: mime_type.to_owned(),
        size,
        sha256: hash.as_bytes().to_vec(),
        ticket: ticket.to_owned(),
    }
}

/// Validates notification artwork without encoding a frame.
pub(crate) fn encode_notification(notification: &Notification) -> wire::Notification {
    wire::Notification {
        id: notification.id.clone(),
        application: notification.application.clone(),
        title: notification.title.clone(),
        body: notification.body.clone(),
        timestamp_ms: notification.timestamp_ms,
        silent: notification.silent,
        kind: match notification.kind {
            NotificationKind::Message => wire::NotificationKind::Message,
            NotificationKind::Media => wire::NotificationKind::Media,
            NotificationKind::Other => wire::NotificationKind::Other,
        }
        .into(),
        app_icon: None,
        image: None,
        actions: notification
            .actions
            .iter()
            .map(|action| wire::NotificationAction {
                id: action.id.clone(),
                label: action.label.clone(),
            })
            .collect(),
    }
}

fn decode_notification_image(
    image: &wire::NotificationImage,
    limit: u64,
) -> Result<BlobTicket, ProtocolError> {
    validate_mime_type(&image.mime_type)
        .map_err(|_| ProtocolError::InvalidNotification("image"))?;
    if image.size == 0 {
        return Err(ProtocolError::InvalidNotification("image"));
    }
    if image.size > limit {
        return Err(ProtocolError::NotificationImageTooLarge);
    }
    let ticket = BlobTicket::from_str(&image.ticket)
        .map_err(|_| ProtocolError::InvalidNotification("image ticket"))?;
    if ticket.recursive() || ticket.hash().as_bytes() != image.sha256.as_slice() {
        return Err(ProtocolError::InvalidNotification("image ticket"));
    }
    Ok(ticket)
}

/// Decodes notification metadata and returns the images to read next.
pub(crate) fn decode_notification_with_images(
    notification: wire::Notification,
) -> Result<DecodedNotification, ProtocolError> {
    let mut images = Vec::new();
    if let Some(icon) = notification.app_icon.as_ref() {
        images.push(decode_notification_image(
            icon,
            MAX_NOTIFICATION_ICON_BYTES,
        )?);
    }
    if let Some(image) = notification.image.as_ref() {
        images.push(decode_notification_image(
            image,
            MAX_NOTIFICATION_IMAGE_BYTES,
        )?);
    }
    Ok(DecodedNotification {
        notification: decode_notification(notification)?,
        images,
    })
}

pub(crate) fn decode_notification(
    notification: wire::Notification,
) -> Result<Notification, ProtocolError> {
    if notification.id.len() > 512 {
        return Err(ProtocolError::InvalidNotification("id"));
    }
    if notification.application.chars().count() > 256 {
        return Err(ProtocolError::InvalidNotification("application"));
    }
    if notification.title.chars().count() > 512 {
        return Err(ProtocolError::InvalidNotification("title"));
    }
    if notification.body.chars().count() > 16 * 1024 {
        return Err(ProtocolError::InvalidNotification("body"));
    }
    if notification.actions.len() > MAX_NOTIFICATION_ACTIONS {
        return Err(ProtocolError::InvalidNotification("actions"));
    }
    for action in &notification.actions {
        if action.id.is_empty() || action.id.len() > MAX_NOTIFICATION_ACTION_ID {
            return Err(ProtocolError::InvalidNotification("action id"));
        }
        if action.label.chars().count() > MAX_NOTIFICATION_ACTION_LABEL {
            return Err(ProtocolError::InvalidNotification("action label"));
        }
    }
    Ok(Notification {
        id: notification.id,
        application: notification.application,
        title: notification.title,
        body: notification.body,
        timestamp_ms: notification.timestamp_ms,
        silent: notification.silent,
        kind: match wire::NotificationKind::try_from(notification.kind) {
            Ok(wire::NotificationKind::Message) => NotificationKind::Message,
            Ok(wire::NotificationKind::Media) => NotificationKind::Media,
            Ok(wire::NotificationKind::Other | wire::NotificationKind::Unspecified) | Err(_) => {
                NotificationKind::Other
            }
        },
        app_icon: notification
            .app_icon
            .as_ref()
            .map(|image| crate::NotificationImage {
                mime_type: image.mime_type.clone(),
                path: String::new(),
            }),
        image: notification
            .image
            .as_ref()
            .map(|image| crate::NotificationImage {
                mime_type: image.mime_type.clone(),
                path: String::new(),
            }),
        actions: notification
            .actions
            .iter()
            .map(|action| crate::NotificationAction {
                id: action.id.clone(),
                label: action.label.clone(),
            })
            .collect(),
    })
}

pub(crate) fn message_name(body: &wire::control_frame::Body) -> &'static str {
    match body {
        wire::control_frame::Body::PairingAuthenticate(_) => "pairing authentication",
        wire::control_frame::Body::PairingAuthenticated(_) => "pairing acknowledgement",
        wire::control_frame::Body::PairingRejected(_) => "pairing rejection",
        wire::control_frame::Body::SessionHello(_) => "session hello",
        wire::control_frame::Body::CapabilityChanged(_) => "capability change",
        wire::control_frame::Body::Notification(_) => "notification",
        wire::control_frame::Body::AddressSnapshot(_) => "address snapshot",
        wire::control_frame::Body::ClipboardItem(_) => "clipboard item",
        wire::control_frame::Body::MediaSnapshot(_) => "media snapshot",
        wire::control_frame::Body::MediaCommand(_) => "media command",
        wire::control_frame::Body::NotificationActionActivated(_) => "notification action",
        wire::control_frame::Body::NotificationConsumed(_) => "notification consumption",
        wire::control_frame::Body::FileOffer(_) => "file offer",
    }
}

/// Validated offer fields received from a trusted peer.
#[derive(Debug)]
pub(crate) struct DecodedFileOffer {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) size: u64,
    pub(crate) ticket: String,
}

/// Encodes a local file share as an offer frame.
pub(crate) fn encode_file_offer(offer: &crate::FileShare) -> wire::FileOffer {
    wire::FileOffer {
        id: offer.id.clone(),
        name: offer.name.clone(),
        size: offer.size,
        ticket: offer.ticket.clone(),
    }
}

/// Decodes an offer from a trusted peer and verifies its ticket shape.
pub(crate) fn decode_file_offer(offer: wire::FileOffer) -> Result<DecodedFileOffer, ProtocolError> {
    if offer.id.is_empty() || offer.id.len() > 128 {
        return Err(ProtocolError::InvalidFileOffer("id"));
    }
    if offer.name.is_empty() || offer.name.len() > 255 {
        return Err(ProtocolError::InvalidFileOffer("name"));
    }
    if offer.size == 0 {
        return Err(ProtocolError::InvalidFileOffer("size"));
    }
    if offer.ticket.is_empty() || offer.ticket.len() > 4096 {
        return Err(ProtocolError::InvalidFileOffer("ticket"));
    }
    let ticket = BlobTicket::from_str(&offer.ticket)
        .map_err(|_| ProtocolError::InvalidFileOffer("ticket"))?;
    if ticket.recursive() {
        return Err(ProtocolError::InvalidFileOffer("ticket"));
    }
    Ok(DecodedFileOffer {
        id: offer.id,
        name: offer.name,
        size: offer.size,
        ticket: offer.ticket,
    })
}

#[cfg(test)]
mod test;

pub(crate) const MAX_CLIPBOARD_TEXT_BYTES: usize = 48 * 1024;
pub(crate) const MAX_CLIPBOARD_MIME_LENGTH: usize = 255;
pub(crate) const MAX_CLIPBOARD_BLOB_BYTES: u64 = 32 * 1024 * 1024;

/// Converts a text clipboard payload into its wire representation.
pub(crate) fn encode_clipboard_text(
    mime_type: &str,
    text: &str,
) -> Result<wire::ClipboardItem, ProtocolError> {
    validate_clipboard_common(mime_type)?;
    if text.is_empty() {
        return Err(ProtocolError::InvalidClipboard("text"));
    }
    if text.len() > MAX_CLIPBOARD_TEXT_BYTES {
        return Err(ProtocolError::InvalidClipboard("text"));
    }
    Ok(wire::ClipboardItem {
        mime_type: mime_type.to_owned(),
        payload: Some(ClipboardPayload::Text(wire::ClipboardText {
            text: text.to_owned(),
        })),
    })
}

/// Converts a blob ticket into its clipboard wire representation.
pub(crate) fn encode_clipboard_blob(
    mime_type: &str,
    size: u64,
    hash: iroh_blobs::Hash,
    ticket: &str,
) -> Result<wire::ClipboardItem, ProtocolError> {
    validate_clipboard_common(mime_type)?;
    Ok(wire::ClipboardItem {
        mime_type: mime_type.to_owned(),
        payload: Some(ClipboardPayload::Blob(wire::ClipboardBlob {
            size,
            sha256: hash.as_bytes().to_vec(),
            ticket: ticket.to_owned(),
        })),
    })
}

/// Decoded clipboard control metadata.
#[derive(Debug)]
pub(crate) enum ClipboardMetadata {
    Text {
        text: String,
    },
    Blob {
        mime_type: String,
        ticket: BlobTicket,
    },
}

pub(crate) fn decode_clipboard_item(
    item: wire::ClipboardItem,
) -> Result<ClipboardMetadata, ProtocolError> {
    validate_clipboard_common(&item.mime_type)?;
    match item.payload {
        Some(ClipboardPayload::Text(text)) => {
            if !item.mime_type.starts_with("text/plain") {
                return Err(ProtocolError::ClipboardPayloadMismatch);
            }
            if text.text.is_empty() || text.text.len() > MAX_CLIPBOARD_TEXT_BYTES {
                return Err(ProtocolError::InvalidClipboard("text"));
            }
            Ok(ClipboardMetadata::Text { text: text.text })
        }
        Some(ClipboardPayload::Blob(blob)) => {
            if item.mime_type.starts_with("text/plain") {
                return Err(ProtocolError::ClipboardPayloadMismatch);
            }
            if blob.size == 0 {
                return Err(ProtocolError::InvalidClipboard("blob size"));
            }
            let ticket = BlobTicket::from_str(&blob.ticket)
                .map_err(|_| ProtocolError::InvalidClipboard("blob ticket"))?;
            if ticket.recursive() || ticket.hash().as_bytes() != blob.sha256.as_slice() {
                return Err(ProtocolError::InvalidClipboard("blob ticket"));
            }
            Ok(ClipboardMetadata::Blob {
                mime_type: item.mime_type,
                ticket,
            })
        }
        None => Err(ProtocolError::ClipboardPayloadMismatch),
    }
}

fn validate_clipboard_common(mime_type: &str) -> Result<(), ProtocolError> {
    validate_mime_type(mime_type).map_err(|_| ProtocolError::InvalidClipboard("mime type"))?;
    if mime_type.len() > MAX_CLIPBOARD_MIME_LENGTH {
        return Err(ProtocolError::InvalidClipboard("mime type"));
    }
    Ok(())
}

pub(crate) fn validate_mime_type(mime_type: &str) -> Result<(), ProtocolError> {
    if mime_type.is_empty()
        || mime_type.len() > 255
        || mime_type.trim() != mime_type
        || !mime_type.contains('/')
        || mime_type.chars().any(char::is_control)
    {
        return Err(ProtocolError::InvalidClipboard("mime type"));
    }
    Ok(())
}

/// Validated media player state carried over the wire.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MediaPlayerState {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) status: crate::PlaybackStatus,
    pub(crate) metadata: crate::MediaMetadata,
    pub(crate) artwork: Option<MediaArtworkRef>,
    pub(crate) capabilities: crate::MediaCapabilities,
    pub(crate) position_ms: Option<u64>,
}

const MAX_MEDIA_ID_LENGTH: usize = 256;
const MAX_MEDIA_NAME_LENGTH: usize = 256;
const MAX_MEDIA_TEXT_LENGTH: usize = 512;
const MAX_MEDIA_ART_URL_LENGTH: usize = 4096;
const MAX_MEDIA_ARTWORK_BYTES: u64 = 4 * 1024 * 1024;

/// Decoded artwork reference carried in a media snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MediaArtworkRef {
    pub(crate) mime_type: String,
    pub(crate) ticket: iroh_blobs::ticket::BlobTicket,
}

fn decode_media_artwork(artwork: wire::MediaArtwork) -> Result<MediaArtworkRef, ProtocolError> {
    validate_mime_type(&artwork.mime_type)
        .map_err(|_| ProtocolError::InvalidMedia("artwork mime type"))?;
    if !artwork.mime_type.starts_with("image/") {
        return Err(ProtocolError::InvalidMedia("artwork mime type"));
    }
    if artwork.size == 0 || artwork.size > MAX_MEDIA_ARTWORK_BYTES {
        return Err(ProtocolError::InvalidMedia("artwork size"));
    }
    let ticket = iroh_blobs::ticket::BlobTicket::from_str(&artwork.ticket)
        .map_err(|_| ProtocolError::InvalidMedia("artwork ticket"))?;
    if ticket.recursive() || ticket.hash().as_bytes() != artwork.sha256.as_slice() {
        return Err(ProtocolError::InvalidMedia("artwork ticket"));
    }
    Ok(MediaArtworkRef {
        mime_type: artwork.mime_type,
        ticket,
    })
}

/// One player plus optional artwork ticket to encode into the snapshot.
pub(crate) struct MediaSnapshotEntry<'a> {
    pub(crate) player: &'a crate::MediaPlayer,
    pub(crate) artwork: Option<crate::protocol::MediaArtworkWire>,
}

/// Artwork already imported into the blob store, ready to describe on the wire.
pub(crate) struct MediaArtworkWire {
    pub(crate) mime_type: String,
    pub(crate) size: u64,
    pub(crate) hash: iroh_blobs::Hash,
    pub(crate) ticket: String,
}

pub(crate) fn encode_media_snapshot(players: &[MediaSnapshotEntry<'_>]) -> wire::MediaSnapshot {
    let players = players
        .iter()
        .filter(|entry| entry.player.origin == crate::MediaOrigin::Local)
        .map(|entry| {
            let player = entry.player;
            wire::MediaState {
                player_id: player.id.clone(),
                name: player.name.clone(),
                status: wire_status(player.status).into(),
                metadata: Some(wire::MediaMetadata {
                    title: player.metadata.title.clone(),
                    artist: player.metadata.artist.clone(),
                    album: player.metadata.album.clone(),
                    art_url: player.metadata.art_url.clone(),
                    duration_ms: player.metadata.duration_ms.unwrap_or_default(),
                    has_duration: player.metadata.duration_ms.is_some(),
                    artwork: entry.artwork.as_ref().map(|artwork| wire::MediaArtwork {
                        mime_type: artwork.mime_type.clone(),
                        size: artwork.size,
                        sha256: artwork.hash.as_bytes().to_vec(),
                        ticket: artwork.ticket.clone(),
                    }),
                }),
                capabilities: Some(wire::MediaCapabilities {
                    can_play: player.capabilities.supports(crate::MediaCommand::Play)
                        || player.capabilities.supports(crate::MediaCommand::PlayPause),
                    can_pause: player.capabilities.supports(crate::MediaCommand::Pause)
                        || player.capabilities.supports(crate::MediaCommand::PlayPause),
                    can_go_next: player.capabilities.supports(crate::MediaCommand::Next),
                    can_go_previous: player.capabilities.supports(crate::MediaCommand::Previous),
                }),
                position_ms: player.position_ms.unwrap_or_default(),
                has_position: player.position_ms.is_some(),
            }
        })
        .collect();
    wire::MediaSnapshot { players }
}

pub(crate) fn decode_media_snapshot(
    snapshot: wire::MediaSnapshot,
) -> Result<Vec<MediaPlayerState>, ProtocolError> {
    snapshot
        .players
        .into_iter()
        .map(decode_media_player)
        .collect()
}

fn decode_media_player(player: wire::MediaState) -> Result<MediaPlayerState, ProtocolError> {
    if player.player_id.is_empty() || player.player_id.len() > MAX_MEDIA_ID_LENGTH {
        return Err(ProtocolError::InvalidMedia("player id"));
    }
    if player.name.chars().count() > MAX_MEDIA_NAME_LENGTH {
        return Err(ProtocolError::InvalidMedia("name"));
    }
    let metadata = player
        .metadata
        .ok_or(ProtocolError::InvalidMedia("metadata"))?;
    for (field, value) in [
        ("title", &metadata.title),
        ("artist", &metadata.artist),
        ("album", &metadata.album),
    ] {
        if value.chars().count() > MAX_MEDIA_TEXT_LENGTH {
            return Err(ProtocolError::InvalidMedia(field));
        }
    }
    if metadata.art_url.len() > MAX_MEDIA_ART_URL_LENGTH {
        return Err(ProtocolError::InvalidMedia("art url"));
    }
    let artwork = metadata.artwork.map(decode_media_artwork).transpose()?;
    let capabilities = player
        .capabilities
        .ok_or(ProtocolError::InvalidMedia("capabilities"))?;
    Ok(MediaPlayerState {
        id: player.player_id,
        name: player.name,
        status: match wire::PlaybackStatus::try_from(player.status) {
            Ok(wire::PlaybackStatus::Playing) => crate::PlaybackStatus::Playing,
            Ok(wire::PlaybackStatus::Paused) => crate::PlaybackStatus::Paused,
            Ok(wire::PlaybackStatus::Stopped) | Err(_) => crate::PlaybackStatus::Stopped,
        },
        metadata: crate::MediaMetadata {
            title: metadata.title,
            artist: metadata.artist,
            album: metadata.album,
            art_url: metadata.art_url,
            artwork: None,
            duration_ms: metadata.has_duration.then_some(metadata.duration_ms),
        },
        artwork,
        capabilities: capabilities_from_wire(capabilities),
        position_ms: player.has_position.then_some(player.position_ms),
    })
}

fn capabilities_from_wire(capabilities: wire::MediaCapabilities) -> crate::MediaCapabilities {
    let mut commands = Vec::new();
    if capabilities.can_play {
        commands.push(crate::MediaCommand::Play);
    }
    if capabilities.can_pause {
        commands.push(crate::MediaCommand::Pause);
    }
    if capabilities.can_go_next {
        commands.push(crate::MediaCommand::Next);
    }
    if capabilities.can_go_previous {
        commands.push(crate::MediaCommand::Previous);
    }
    crate::MediaCapabilities { commands }
}

fn wire_status(status: crate::PlaybackStatus) -> wire::PlaybackStatus {
    match status {
        crate::PlaybackStatus::Playing => wire::PlaybackStatus::Playing,
        crate::PlaybackStatus::Paused => wire::PlaybackStatus::Paused,
        crate::PlaybackStatus::Stopped => wire::PlaybackStatus::Stopped,
    }
}

pub(crate) fn encode_media_command(
    player_id: &str,
    command: crate::MediaCommand,
) -> wire::MediaCommandRequest {
    wire::MediaCommandRequest {
        player_id: player_id.to_owned(),
        command: match command {
            crate::MediaCommand::Play => wire::MediaCommand::Play,
            crate::MediaCommand::Pause => wire::MediaCommand::Pause,
            crate::MediaCommand::PlayPause => wire::MediaCommand::PlayPause,
            crate::MediaCommand::Stop => wire::MediaCommand::Stop,
            crate::MediaCommand::Next => wire::MediaCommand::Next,
            crate::MediaCommand::Previous => wire::MediaCommand::Previous,
        }
        .into(),
    }
}

pub(crate) fn decode_media_command(
    request: wire::MediaCommandRequest,
) -> Result<(String, crate::MediaCommand), ProtocolError> {
    if request.player_id.is_empty() || request.player_id.len() > MAX_MEDIA_ID_LENGTH {
        return Err(ProtocolError::InvalidMedia("player id"));
    }
    let command = match wire::MediaCommand::try_from(request.command) {
        Ok(wire::MediaCommand::Play) => crate::MediaCommand::Play,
        Ok(wire::MediaCommand::Pause) => crate::MediaCommand::Pause,
        Ok(wire::MediaCommand::PlayPause) => crate::MediaCommand::PlayPause,
        Ok(wire::MediaCommand::Stop) => crate::MediaCommand::Stop,
        Ok(wire::MediaCommand::Next) => crate::MediaCommand::Next,
        Ok(wire::MediaCommand::Previous) => crate::MediaCommand::Previous,
        Err(_) => return Err(ProtocolError::InvalidMedia("command")),
    };
    Ok((request.player_id, command))
}
