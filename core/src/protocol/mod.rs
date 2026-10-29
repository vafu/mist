//! Private contracts and generated types for Mist's versioned wire protocol.
//!
//! Wire types must not cross public Rust or `UniFFI` boundaries. The codec converts
//! them to domain values and limits control frames to 64 KiB of Protobuf payload,
//! excluding the length prefix.

mod codec;

use thiserror::Error;

pub(crate) use codec::{
    decode_session_hello, read_control_frame, session_hello, write_control_frame,
};

pub(crate) const SESSION_PROTOCOL_VERSION: u32 = 1;
const MAX_CONTROL_FRAME_SIZE: usize = 64 * 1024;

#[allow(
    clippy::doc_markdown,
    clippy::trivially_copy_pass_by_ref,
    reason = "generated Protobuf code"
)]
pub(crate) mod wire {
    include!(concat!(env!("OUT_DIR"), "/mist.v1.rs"));
}

#[derive(Debug, Error)]
pub(crate) enum ProtocolError {
    #[error("control frame payload exceeds the {MAX_CONTROL_FRAME_SIZE}-byte limit")]
    FrameTooLarge,
    #[error("invalid control frame length: {0}")]
    InvalidLength(String),
    #[error("failed to decode control frame: {0}")]
    Decode(#[source] prost::DecodeError),
    #[error("failed to read control frame: {0}")]
    Read(String),
    #[error("failed to write control frame: {0}")]
    Write(String),
    #[error("session protocol version {0} is not supported")]
    UnsupportedSessionVersion(u32),
    #[error("session hello did not contain device information")]
    MissingDeviceInfo,
    #[error("device display name must contain between 1 and 128 characters")]
    InvalidDisplayName,
}

#[cfg(test)]
mod test;
