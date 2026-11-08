//! Private generated types for Mist's versioned wire protocol.
//!
//! These types must not cross the crate's public or `UniFFI` boundaries. Public
//! domain types are converted to and from this representation explicitly.

use std::net::SocketAddr;
use std::str::FromStr;

use iroh::endpoint::{RecvStream, SendStream};
use iroh::{EndpointAddr, RelayUrl, TransportAddr};
use iroh_base::CustomAddr;
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
    #[error("session hello did not contain an address snapshot")]
    MissingAddresses,
    #[error("invalid transport address: {0}")]
    InvalidAddress(String),
    #[error("clipboard field {0} exceeds its protocol limit")]
    InvalidClipboard(&'static str),
    #[error("clipboard payload does not match its MIME type")]
    ClipboardPayloadMismatch,
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
    }
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
    }
}

#[cfg(test)]
mod test;

pub(crate) const MAX_CLIPBOARD_TEXT_BYTES: usize = 48 * 1024;
pub(crate) const MAX_CLIPBOARD_MIME_LENGTH: usize = 255;

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

/// Converts blob metadata into its wire representation.
pub(crate) fn encode_clipboard_blob(
    metadata: &crate::transfer::BlobMetadata,
) -> Result<wire::ClipboardItem, ProtocolError> {
    validate_clipboard_common(&metadata.mime_type)?;
    Ok(wire::ClipboardItem {
        mime_type: metadata.mime_type.clone(),
        payload: Some(ClipboardPayload::Blob(wire::ClipboardBlob {
            size: metadata.size,
            sha256: metadata.sha256.to_vec(),
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
        size: u64,
        sha256: [u8; 32],
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
            crate::transfer::validate_size(blob.size)
                .map_err(|_| ProtocolError::InvalidClipboard("blob size"))?;
            let sha256: [u8; 32] = blob
                .sha256
                .as_slice()
                .try_into()
                .map_err(|_| ProtocolError::InvalidClipboard("blob sha256"))?;
            Ok(ClipboardMetadata::Blob {
                mime_type: item.mime_type,
                size: blob.size,
                sha256,
            })
        }
        None => Err(ProtocolError::ClipboardPayloadMismatch),
    }
}

fn validate_clipboard_common(mime_type: &str) -> Result<(), ProtocolError> {
    crate::transfer::validate_mime_type(mime_type)
        .map_err(|_| ProtocolError::InvalidClipboard("mime type"))?;
    if mime_type.len() > MAX_CLIPBOARD_MIME_LENGTH {
        return Err(ProtocolError::InvalidClipboard("mime type"));
    }
    Ok(())
}
