//! Platform media integration for the Linux host.
//!
//! Inbound direction: MPRIS players on this machine are discovered and
//! controlled over D-Bus.
//! Outbound direction: remote player state is exported as an MPRIS player so
//! desktop media controls and `playerctl` can drive the remote device.

mod mpris;
mod publish;

pub use mpris::MprisMedia;
pub use publish::MprisPublisher;
