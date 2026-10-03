//! Private generated types for Mist's versioned wire protocol.
//!
//! These types must not cross the crate's public or `UniFFI` boundaries. Public
//! domain types are converted to and from this representation explicitly.

use iroh::endpoint::{RecvStream, SendStream};
use prost::Message;
use thiserror::Error;

use crate::{DeviceClass, DeviceInfo, Platform};

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

pub(crate) fn session_hello(device: &DeviceInfo) -> wire::SessionHello {
    wire::SessionHello {
        protocol_version: SESSION_PROTOCOL_VERSION,
        device: Some(encode_device(device)),
        capabilities: Some(wire::CapabilitySnapshot {
            capabilities: Vec::new(),
        }),
    }
}

pub(crate) fn decode_session_hello(hello: wire::SessionHello) -> Result<DeviceInfo, ProtocolError> {
    if hello.protocol_version != SESSION_PROTOCOL_VERSION {
        return Err(ProtocolError::UnsupportedSessionVersion(
            hello.protocol_version,
        ));
    }
    let _capabilities = hello
        .capabilities
        .ok_or(ProtocolError::MissingCapabilities)?;
    decode_device(
        hello
            .device
            .as_ref()
            .ok_or(ProtocolError::MissingDeviceInfo)?,
    )
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

pub(crate) fn message_name(body: &wire::control_frame::Body) -> &'static str {
    match body {
        wire::control_frame::Body::PairingAuthenticate(_) => "pairing authentication",
        wire::control_frame::Body::PairingAuthenticated(_) => "pairing acknowledgement",
        wire::control_frame::Body::PairingRejected(_) => "pairing rejection",
        wire::control_frame::Body::SessionHello(_) => "session hello",
        wire::control_frame::Body::CapabilityChanged(_) => "capability change",
    }
}

#[cfg(test)]
mod test;
