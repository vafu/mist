use std::sync::Arc;

use async_trait::async_trait;
use mist_app_contract::{Media, MediaCommand, MediaError, MediaEventStream, MediaPlayer};

use super::RuntimeMediaEventStream;
use crate::runtime::Session;

#[derive(Clone, Debug)]
pub(crate) struct IrohMedia {
    session: Arc<Session>,
}

impl IrohMedia {
    pub(crate) fn new(session: Arc<Session>) -> Self {
        Self { session }
    }
}

#[async_trait]
impl Media for IrohMedia {
    fn list(&self) -> Vec<MediaPlayer> {
        self.session.media_players()
    }

    fn subscribe(&self) -> Arc<dyn MediaEventStream> {
        RuntimeMediaEventStream::new(self.session.subscribe_media())
    }

    fn publish_local(&self, players: Vec<MediaPlayer>) -> Result<(), MediaError> {
        self.session.publish_local_media(players);
        Ok(())
    }

    async fn command(&self, player_id: String, command: MediaCommand) -> Result<(), MediaError> {
        let session = Arc::clone(&self.session);
        let executor = session.executor().clone();
        executor
            .spawn(async move { session.send_media_command(player_id, command).await })
            .await
            .map_err(|error| MediaError::Transport {
                reason: error.to_string(),
            })?
    }
}
