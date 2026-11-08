//! Bidirectional notification implementation.

mod service;

use std::sync::Arc;

use async_trait::async_trait;
use mist_app_contract::{NotificationError, NotificationEventStream, ReceivedNotification};
use tokio::sync::{Mutex, broadcast};

pub(crate) use service::IrohNotificationChannel;

#[derive(Debug)]
pub(crate) struct RuntimeNotificationEventStream {
    receiver: Arc<Mutex<broadcast::Receiver<ReceivedNotification>>>,
}

impl RuntimeNotificationEventStream {
    pub(crate) fn new(receiver: broadcast::Receiver<ReceivedNotification>) -> Arc<Self> {
        Arc::new(Self {
            receiver: Arc::new(Mutex::new(receiver)),
        })
    }
}

#[async_trait]
impl NotificationEventStream for RuntimeNotificationEventStream {
    async fn next(&self) -> Result<ReceivedNotification, NotificationError> {
        self.receiver
            .lock()
            .await
            .recv()
            .await
            .map_err(|error| match error {
                broadcast::error::RecvError::Closed => NotificationError::Closed,
                broadcast::error::RecvError::Lagged(count) => NotificationError::Lagged { count },
            })
    }
}
