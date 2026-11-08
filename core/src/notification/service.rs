use std::sync::Arc;

use async_trait::async_trait;

use mist_app_contract::{
    Notification, NotificationChannel, NotificationError, NotificationEventStream,
};

use super::RuntimeNotificationEventStream;
use crate::runtime::Session;

#[derive(Clone, Debug)]
pub(crate) struct IrohNotificationChannel {
    session: Arc<Session>,
}

impl IrohNotificationChannel {
    pub(crate) fn new(session: Arc<Session>) -> Self {
        Self { session }
    }
}

#[async_trait]
impl NotificationChannel for IrohNotificationChannel {
    async fn push(
        &self,
        notification: Notification,
        targets: Vec<String>,
    ) -> Result<(), NotificationError> {
        self.session.push_notification(&notification, targets)
    }

    fn subscribe(&self) -> Arc<dyn NotificationEventStream> {
        RuntimeNotificationEventStream::new(self.session.subscribe_notifications())
    }
}
