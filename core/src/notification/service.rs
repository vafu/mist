use std::sync::Arc;

use async_trait::async_trait;

use mist_app_contract::{
    Notification, NotificationActivationStream, NotificationChannel, NotificationError,
    NotificationEventStream,
};

use super::{RuntimeNotificationActivationStream, RuntimeNotificationEventStream};
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

    async fn activate(
        &self,
        source_endpoint_id: String,
        notification_id: String,
        action_id: String,
    ) -> Result<(), NotificationError> {
        let endpoint_id = std::str::FromStr::from_str(&source_endpoint_id).map_err(|error| {
            NotificationError::InvalidEndpointId {
                reason: format!("{error}"),
            }
        })?;
        let session = Arc::clone(&self.session);
        let executor = session.executor().clone();
        executor
            .spawn(async move {
                session
                    .activate_notification(endpoint_id, notification_id, action_id)
                    .await
            })
            .await
            .map_err(|error| NotificationError::Executor {
                reason: error.to_string(),
            })?
    }

    fn subscribe_activations(&self) -> Arc<dyn NotificationActivationStream> {
        RuntimeNotificationActivationStream::new(self.session.subscribe_activations())
    }
}
