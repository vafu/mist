use std::sync::Arc;

use async_trait::async_trait;
use mist_app_contract::{Clipboard, ClipboardContent, ClipboardError, ClipboardEventStream};

use super::RuntimeClipboardEventStream;
use crate::runtime::Session;

#[derive(Clone, Debug)]
pub(crate) struct IrohClipboard {
    session: Arc<Session>,
}

impl IrohClipboard {
    pub(crate) fn new(session: Arc<Session>) -> Self {
        Self { session }
    }
}

#[async_trait]
impl Clipboard for IrohClipboard {
    async fn push(
        &self,
        content: ClipboardContent,
        targets: Vec<String>,
    ) -> Result<(), ClipboardError> {
        self.session.push_clipboard(&content, targets).await
    }

    fn subscribe(&self) -> Arc<dyn ClipboardEventStream> {
        RuntimeClipboardEventStream::new(self.session.subscribe_clipboard())
    }
}
