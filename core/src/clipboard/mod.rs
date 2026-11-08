//! Bidirectional clipboard sharing over the authenticated Mist session.

mod service;

use std::sync::Arc;

use async_trait::async_trait;
use mist_app_contract::{ClipboardError, ClipboardEventStream, ReceivedClipboard};
use tokio::sync::{Mutex, broadcast};

pub(crate) use service::IrohClipboard;

/// Reactive clipboard stream.
///
/// The receiver is awaited directly rather than in a detached task so that
/// cancelling `next` (for example with a timeout) cannot keep the receiver
/// locked and consume the following event.
#[derive(Debug)]
pub(crate) struct RuntimeClipboardEventStream {
    receiver: Arc<Mutex<broadcast::Receiver<ReceivedClipboard>>>,
}

impl RuntimeClipboardEventStream {
    pub(crate) fn new(receiver: broadcast::Receiver<ReceivedClipboard>) -> Arc<Self> {
        Arc::new(Self {
            receiver: Arc::new(Mutex::new(receiver)),
        })
    }
}

#[async_trait]
impl ClipboardEventStream for RuntimeClipboardEventStream {
    async fn next(&self) -> Result<ReceivedClipboard, ClipboardError> {
        self.receiver
            .lock()
            .await
            .recv()
            .await
            .map_err(|error| match error {
                broadcast::error::RecvError::Closed => ClipboardError::Closed,
                broadcast::error::RecvError::Lagged(count) => ClipboardError::Lagged { count },
            })
    }
}
