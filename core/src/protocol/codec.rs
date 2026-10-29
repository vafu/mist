use iroh::endpoint::{RecvStream, SendStream};
use prost::Message;

use super::{MAX_CONTROL_FRAME_SIZE, ProtocolError, SESSION_PROTOCOL_VERSION, wire};
use crate::{DeviceInfo, Platform};

pub(crate) async fn write_control_frame(
    stream: &mut SendStream,
    frame: &wire::ControlFrame,
) -> Result<(), ProtocolError> {
    if frame.encoded_len() > MAX_CONTROL_FRAME_SIZE {
        return Err(ProtocolError::FrameTooLarge);
    }
    let encoded = frame.encode_length_delimited_to_vec();
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
    }
}

pub(crate) fn decode_session_hello(
    hello: &wire::SessionHello,
) -> Result<DeviceInfo, ProtocolError> {
    if hello.protocol_version != SESSION_PROTOCOL_VERSION {
        return Err(ProtocolError::UnsupportedSessionVersion(
            hello.protocol_version,
        ));
    }
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
        platform: match device.platform {
            Platform::Android => wire::Platform::Android,
            Platform::Linux => wire::Platform::Linux,
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
        platform: match wire::Platform::try_from(device.platform) {
            Ok(wire::Platform::Android) => Platform::Android,
            Ok(wire::Platform::Linux) => Platform::Linux,
            Ok(wire::Platform::Unspecified | wire::Platform::Other) | Err(_) => Platform::Other,
        },
    })
}
