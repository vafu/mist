use std::sync::Arc;

use async_trait::async_trait;

use mist_app_contract::{
    Notification, NotificationActivationStream, NotificationChannel, NotificationConsumedStream,
    NotificationError, NotificationEventStream,
};

use super::{
    RuntimeNotificationActivationStream, RuntimeNotificationConsumedStream,
    RuntimeNotificationEventStream,
};
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
        self.session.push_notification(&notification, targets).await
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

    async fn consume(
        &self,
        source_endpoint_id: String,
        notification_id: String,
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
                    .consume_notification(endpoint_id, notification_id)
                    .await
            })
            .await
            .map_err(|error| NotificationError::Executor {
                reason: error.to_string(),
            })?
    }

    async fn retract(&self, notification_id: String) -> Result<(), NotificationError> {
        let session = Arc::clone(&self.session);
        let executor = session.executor().clone();
        executor
            .spawn(async move { session.retract_notification(notification_id).await })
            .await
            .map_err(|error| NotificationError::Executor {
                reason: error.to_string(),
            })?
    }

    fn subscribe_activations(&self) -> Arc<dyn NotificationActivationStream> {
        RuntimeNotificationActivationStream::new(self.session.subscribe_activations())
    }

    fn subscribe_consumed(&self) -> Arc<dyn NotificationConsumedStream> {
        RuntimeNotificationConsumedStream::new(self.session.subscribe_consumed())
    }
}
