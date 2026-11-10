//! In-memory outbound scheduling and echo suppression.
//!
//! Sends are never discarded because a peer is temporarily unreachable: they are
//! queued per endpoint and flushed when a connection is installed. The queue is
//! process-local and intentionally not persisted.
//!
//! Echo suppression also lives here, at the message layer: content this runtime
//! received from a peer is fingerprinted, and a subsequent identical outbound
//! message is dropped once. Features and applications do not participate.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Mutex, PoisonError};

use iroh::EndpointId;
use iroh::endpoint::Connection;

use crate::protocol::wire::{ControlFrame, control_frame};
use crate::protocol::{
    encode_clipboard_blob, encode_clipboard_text, encode_file_offer, encode_notification,
    wire_image, write_control_frame,
};
use crate::{ClipboardContent, FileShare, Notification};
use iroh_blobs::ticket::BlobTicket;

/// One scheduled outbound message.
#[derive(Debug)]
pub(crate) enum Outbound {
    Notification {
        notification: Notification,
        images: Vec<Option<BlobAttachment>>,
    },
    Clipboard {
        content: ClipboardContent,
        blob: Option<ClipboardBlob>,
    },
    FileOffer(FileShare),
}

#[derive(Clone, Debug)]
pub(crate) struct BlobAttachment {
    pub(crate) mime_type: String,
    pub(crate) size: u64,
    pub(crate) hash: iroh_blobs::Hash,
    pub(crate) ticket: BlobTicket,
}

pub(crate) use BlobAttachment as ClipboardBlob;

impl Outbound {
    /// Stable content fingerprint used for echo suppression.
    ///
    /// Returns `None` for messages that cannot echo, such as notifications.
    pub(crate) fn fingerprint(&self) -> Option<[u8; 32]> {
        match self {
            Self::Notification { .. } | Self::FileOffer(_) => None,
            Self::Clipboard { content, blob } => Some(match (content, blob) {
                (ClipboardContent::Text { text }, _) => hash_parts(&[b"text", text.as_bytes()]),
                (ClipboardContent::Blob { .. }, Some(blob)) => *blob.hash.as_bytes(),
                (ClipboardContent::Blob { mime_type, .. }, None) => {
                    hash_parts(&[b"blob", mime_type.as_bytes()])
                }
            }),
        }
    }
}

/// Fingerprint for inbound text content, matching [`Outbound::fingerprint`].
pub(crate) fn text_fingerprint(text: &str) -> [u8; 32] {
    hash_parts(&[b"text", text.as_bytes()])
}

fn hash_parts(parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    for part in parts {
        hasher.update(&(part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    *hasher.finalize().as_bytes()
}

/// Boundary of the recently received content window.
const MAX_RECENT_ECHOES: usize = 64;

/// Per-endpoint FIFO of messages awaiting a connection.
#[derive(Debug, Default)]
pub(crate) struct Outbox {
    pending: Mutex<HashMap<EndpointId, VecDeque<Outbound>>>,
    flushing: Mutex<HashSet<EndpointId>>,
    recent: Mutex<VecDeque<[u8; 32]>>,
}

impl Outbox {
    pub(crate) fn push(&self, endpoint_id: EndpointId, message: Outbound) {
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(endpoint_id)
            .or_default()
            .push_back(message);
    }

    pub(crate) fn push_front(&self, endpoint_id: EndpointId, message: Outbound) {
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(endpoint_id)
            .or_default()
            .push_front(message);
    }

    pub(crate) fn pop(&self, endpoint_id: EndpointId) -> Option<Outbound> {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let queue = pending.get_mut(&endpoint_id)?;
        let message = queue.pop_front();
        if queue.is_empty() {
            pending.remove(&endpoint_id);
        }
        message
    }

    pub(crate) fn has_pending(&self, endpoint_id: EndpointId) -> bool {
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(&endpoint_id)
    }

    /// Claims the flush slot for an endpoint.
    ///
    /// Returns `false` when another flush is already draining the queue, which
    /// keeps ordering intact without blocking.
    pub(crate) fn begin_flush(&self, endpoint_id: EndpointId) -> bool {
        self.flushing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(endpoint_id)
    }

    pub(crate) fn end_flush(&self, endpoint_id: EndpointId) {
        self.flushing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&endpoint_id);
    }

    /// Records content that arrived from a peer.
    pub(crate) fn remember_received(&self, fingerprint: [u8; 32]) {
        let mut recent = self.recent.lock().unwrap_or_else(PoisonError::into_inner);
        if recent.len() >= MAX_RECENT_ECHOES {
            recent.pop_front();
        }
        if !recent.contains(&fingerprint) {
            recent.push_back(fingerprint);
        }
    }

    /// Returns true when `fingerprint` is an echo of received content.
    ///
    /// The entry is consumed so identical content sent later can still travel.
    pub(crate) fn suppress_echo(&self, fingerprint: [u8; 32]) -> bool {
        let mut recent = self.recent.lock().unwrap_or_else(PoisonError::into_inner);
        match recent.iter().position(|entry| *entry == fingerprint) {
            Some(index) => {
                recent.remove(index);
                true
            }
            None => false,
        }
    }
}

/// Writes one outbound message to a live connection.
pub(crate) async fn send(connection: &Connection, message: &Outbound) -> Result<(), String> {
    let mut stream = connection
        .open_uni()
        .await
        .map_err(|error| error.to_string())?;
    match message {
        Outbound::Notification {
            notification,
            images,
        } => {
            let mut frame = encode_notification(notification);
            let icon = images.first().and_then(Option::as_ref);
            let image = images.get(1).and_then(Option::as_ref);
            frame.app_icon = icon.map(|attachment| {
                wire_image(
                    &attachment.mime_type,
                    attachment.size,
                    attachment.hash,
                    &attachment.ticket.to_string(),
                )
            });
            frame.image = image.map(|attachment| {
                wire_image(
                    &attachment.mime_type,
                    attachment.size,
                    attachment.hash,
                    &attachment.ticket.to_string(),
                )
            });
            write_control_frame(
                &mut stream,
                &ControlFrame {
                    body: Some(control_frame::Body::Notification(frame)),
                },
            )
            .await
            .map_err(|error| error.to_string())?;
        }
        Outbound::Clipboard { content, blob } => {
            let frame = match (content, blob) {
                (ClipboardContent::Text { text }, _) => {
                    encode_clipboard_text("text/plain;charset=utf-8", text)
                }
                (ClipboardContent::Blob { .. }, Some(blob)) => encode_clipboard_blob(
                    &blob.mime_type,
                    blob.size,
                    blob.hash,
                    &blob.ticket.to_string(),
                ),
                (ClipboardContent::Blob { .. }, None) => {
                    return Err("clipboard blob metadata is missing".to_owned());
                }
            }
            .map_err(|error| error.to_string())?;
            write_control_frame(
                &mut stream,
                &ControlFrame {
                    body: Some(control_frame::Body::ClipboardItem(frame)),
                },
            )
            .await
            .map_err(|error| error.to_string())?;
        }
        Outbound::FileOffer(offer) => {
            write_control_frame(
                &mut stream,
                &ControlFrame {
                    body: Some(control_frame::Body::FileOffer(encode_file_offer(offer))),
                },
            )
            .await
            .map_err(|error| error.to_string())?;
        }
    }
    stream.finish().map_err(|error| error.to_string())
}
