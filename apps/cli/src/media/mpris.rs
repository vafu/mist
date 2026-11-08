use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use futures_util::StreamExt;
use mist_app_contract::{MediaCommand, MediaMetadata, PlaybackStatus};
use mist_core::host::{LocalPlayer, MediaProvider, MediaProviderError, MediaWatcher};
use zbus::zvariant::OwnedValue;
use zbus::{Connection, MatchRule, MessageStream, Proxy};

const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";
const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
const MPRIS_PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const MPRIS_ROOT: &str = "org.mpris.MediaPlayer2";
const MIST_PLAYER_NAME: &str = "org.mpris.MediaPlayer2.mist";

/// Linux MPRIS integration.
pub struct MprisMedia {
    connection: Connection,
}

impl fmt::Debug for MprisMedia {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("MprisMedia").finish_non_exhaustive()
    }
}

impl MprisMedia {
    /// Connects to the session bus.
    ///
    /// # Errors
    ///
    /// Returns an error when the session bus is unavailable.
    pub async fn new() -> Result<Arc<Self>, MediaProviderError> {
        let connection =
            Connection::session()
                .await
                .map_err(|error| MediaProviderError::Unavailable {
                    reason: error.to_string(),
                })?;
        Ok(Arc::new(Self { connection }))
    }
}

#[async_trait::async_trait]
impl MediaProvider for MprisMedia {
    fn watch(&self) -> Result<Box<dyn MediaWatcher>, MediaProviderError> {
        MprisWatcher::new(self.connection.clone()).map(|watcher| Box::new(watcher) as Box<_>)
    }

    async fn command(
        &self,
        player_id: &str,
        command: MediaCommand,
    ) -> Result<(), MediaProviderError> {
        let connection = self.connection.clone();
        let bus_name = format!("{MPRIS_PREFIX}{player_id}");
        let method = match command {
            MediaCommand::Play => "Play",
            MediaCommand::Pause => "Pause",
            MediaCommand::PlayPause => "PlayPause",
            MediaCommand::Stop => "Stop",
            MediaCommand::Next => "Next",
            MediaCommand::Previous => "Previous",
        };
        let proxy = Proxy::new(&connection, bus_name, MPRIS_PATH, MPRIS_PLAYER)
            .await
            .map_err(|error| MediaProviderError::Command {
                reason: error.to_string(),
            })?;
        proxy
            .call_method(method, &())
            .await
            .map(|_| ())
            .map_err(|error| MediaProviderError::Command {
                reason: error.to_string(),
            })
    }
}

struct MprisWatcher {
    runtime: tokio::runtime::Runtime,
    connection: Connection,
    changes: tokio::sync::mpsc::UnboundedReceiver<()>,
}

impl MprisWatcher {
    fn new(connection: Connection) -> Result<Self, MediaProviderError> {
        let runtime = watcher_runtime()?;
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let watched = connection.clone();
        runtime.spawn(async move {
            if watch_signals(&watched, sender).await.is_err() {
                // The consumer observes a closed channel and stops.
            }
        });
        Ok(Self {
            runtime,
            connection,
            changes: receiver,
        })
    }
}

async fn watch_signals(
    connection: &Connection,
    sender: tokio::sync::mpsc::UnboundedSender<()>,
) -> Result<(), zbus::Error> {
    let name_rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.freedesktop.DBus")?
        .member("NameOwnerChanged")?
        .build();
    let property_rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.freedesktop.DBus.Properties")?
        .member("PropertiesChanged")?
        .build();

    let mut names = MessageStream::for_match_rule(name_rule, connection, None).await?;
    let mut properties = MessageStream::for_match_rule(property_rule, connection, None).await?;

    loop {
        tokio::select! {
            message = names.next() => {
                if message.is_none() {
                    return Ok(());
                }
            }
            message = properties.next() => {
                match message {
                    Some(Ok(message)) => {
                        if !is_mpris_message(&message) {
                            continue;
                        }
                    }
                    Some(Err(_)) | None => return Ok(()),
                }
            }
        }
        if sender.send(()).is_err() {
            return Ok(());
        }
    }
}

fn is_mpris_message(message: &zbus::Message) -> bool {
    message
        .header()
        .path()
        .and_then(|path| path.as_str().strip_prefix(MPRIS_PATH))
        .is_some()
}

fn watcher_runtime() -> Result<tokio::runtime::Runtime, MediaProviderError> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| MediaProviderError::Unavailable {
            reason: error.to_string(),
        })
}

impl MediaWatcher for MprisWatcher {
    fn next(&mut self) -> Result<Option<Vec<LocalPlayer>>, MediaProviderError> {
        let received = self.runtime.block_on(self.changes.recv());
        if received.is_none() {
            return Ok(None);
        }
        let connection = self.connection.clone();
        let players = self
            .runtime
            .block_on(async move { collect_players(&connection).await })
            .map_err(|error| MediaProviderError::Unavailable {
                reason: error.to_string(),
            })?;
        Ok(Some(players))
    }
}

async fn collect_players(connection: &Connection) -> Result<Vec<LocalPlayer>, zbus::Error> {
    let names = connection
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "ListNames",
            &(),
        )
        .await?
        .body()
        .deserialize::<Vec<String>>()?;

    let mut players = Vec::new();
    for name in names {
        let Some(player_id) = name.strip_prefix(MPRIS_PREFIX) else {
            continue;
        };
        if name == MIST_PLAYER_NAME || name.starts_with("org.mpris.MediaPlayer2.mist.") {
            continue;
        }
        if let Ok(player) = read_player(connection, player_id).await {
            players.push(player);
        }
    }
    players.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(players)
}

async fn read_player(connection: &Connection, player_id: &str) -> Result<LocalPlayer, zbus::Error> {
    let bus_name = format!("{MPRIS_PREFIX}{player_id}");
    let player = Proxy::new(connection, bus_name.clone(), MPRIS_PATH, MPRIS_PLAYER).await?;
    let root = Proxy::new(connection, bus_name, MPRIS_PATH, MPRIS_ROOT).await?;

    let metadata: HashMap<String, OwnedValue> =
        player.get_property("Metadata").await.unwrap_or_default();
    let status: String = player
        .get_property("PlaybackStatus")
        .await
        .unwrap_or_else(|_| "Stopped".to_owned());
    let name: String = root
        .get_property("Identity")
        .await
        .unwrap_or_else(|_| player_id.to_owned());
    let can_play: bool = player.get_property("CanPlay").await.unwrap_or(false);
    let can_pause: bool = player.get_property("CanPause").await.unwrap_or(false);
    let can_go_next: bool = player.get_property("CanGoNext").await.unwrap_or(false);
    let can_go_previous: bool = player.get_property("CanGoPrevious").await.unwrap_or(false);

    let mut commands = Vec::new();
    if can_play {
        commands.push(MediaCommand::Play);
    }
    if can_pause {
        commands.push(MediaCommand::Pause);
    }
    if can_go_next {
        commands.push(MediaCommand::Next);
    }
    if can_go_previous {
        commands.push(MediaCommand::Previous);
    }

    Ok(LocalPlayer {
        id: player_id.to_owned(),
        name,
        status: match status.as_str() {
            "Playing" => PlaybackStatus::Playing,
            "Paused" => PlaybackStatus::Paused,
            _ => PlaybackStatus::Stopped,
        },
        metadata: MediaMetadata {
            title: metadata_string(&metadata, "xesam:title"),
            artist: metadata_string_list(&metadata, "xesam:artist"),
            album: metadata_string(&metadata, "xesam:album"),
            art_url: metadata_string(&metadata, "mpris:artUrl"),
            duration_ms: metadata_i64(&metadata, "mpris:length")
                .and_then(|value| u64::try_from(value / 1000).ok()),
        },
        commands,
        position_ms: None,
    })
}

fn metadata_string(metadata: &HashMap<String, OwnedValue>, key: &str) -> String {
    metadata
        .get(key)
        .and_then(|value| <&str>::try_from(value).ok())
        .map_or_else(String::new, ToOwned::to_owned)
}

fn metadata_string_list(metadata: &HashMap<String, OwnedValue>, key: &str) -> String {
    let Some(value) = metadata.get(key) else {
        return String::new();
    };
    let Ok(values) = value.downcast_ref::<zbus::zvariant::Array>() else {
        return String::new();
    };
    values
        .iter()
        .filter_map(|entry| entry.downcast_ref::<&str>().ok().map(ToOwned::to_owned))
        .collect::<Vec<String>>()
        .join(", ")
}

fn metadata_i64(metadata: &HashMap<String, OwnedValue>, key: &str) -> Option<i64> {
    metadata
        .get(key)
        .and_then(|value| i64::try_from(value).ok())
}
