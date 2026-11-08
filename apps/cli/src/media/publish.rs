//! Exports remote media players as local MPRIS players.
//!
//! Each remote player is exported as `org.mpris.MediaPlayer2.mist.<id>`, so
//! desktop media widgets and `playerctl` can drive the paired device.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use mist_app_contract::{Media, MediaCommand, MediaEventKind, MediaOrigin, MediaPlayer};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};
use zbus::{Connection, interface};

const MPRIS_PATH: &str = "/org/mpris/MediaPlayer2";
const MIST_PREFIX: &str = "org.mpris.MediaPlayer2.mist.";

/// Tracks exported MPRIS players and keeps them in sync with the runtime.
#[derive(Debug)]
pub struct MprisPublisher {
    connection: Connection,
    media: Arc<dyn Media>,
    published: Mutex<HashMap<String, MediaPlayer>>,
}

impl MprisPublisher {
    /// Creates a publisher on an existing session connection.
    #[must_use]
    pub fn new(connection: Connection, media: Arc<dyn Media>) -> Self {
        Self {
            connection,
            media,
            published: Mutex::new(HashMap::new()),
        }
    }

    /// Watches runtime media events and exports/updates/removes MPRIS players.
    pub async fn run(self: Arc<Self>) {
        let events = self.media.subscribe();
        loop {
            let Ok(event) = events.next().await else {
                return;
            };
            if event.player.origin != MediaOrigin::Remote {
                continue;
            }
            match event.kind {
                MediaEventKind::Added | MediaEventKind::Changed => {
                    self.upsert(event.player).await;
                }
                MediaEventKind::Removed => self.remove(&event.player.id),
            }
        }
    }

    async fn upsert(self: &Arc<Self>, player: MediaPlayer) {
        let existing = self
            .published
            .lock()
            .map(|published| published.get(&player.id).cloned())
            .unwrap_or(None);
        let state = Arc::new(Mutex::new(player.clone()));

        if existing.is_some() {
            if let Ok(interface) = self
                .connection
                .object_server()
                .interface::<_, MistPlayerControls>(MPRIS_PATH)
                .await
            {
                interface.get().await.replace(player.clone());
                let emitter = interface.signal_emitter().to_owned();
                drop(interface);
                emit_changed(&emitter, &player).await;
            }
        } else {
            let bus_name = format!("{MIST_PREFIX}{}", sanitize(&player.id));
            let _ = self.connection.request_name(bus_name).await;
            let controls = MistPlayerControls {
                state: Arc::clone(&state),
                media: Arc::clone(&self.media),
            };
            let root = MistPlayerRoot {
                state: Arc::clone(&state),
            };
            let server = self.connection.object_server();
            let _ = server.at(MPRIS_PATH, controls).await;
            let _ = server.at(MPRIS_PATH, root).await;
            if let Ok(interface) = server.interface::<_, MistPlayerControls>(MPRIS_PATH).await {
                let emitter = interface.signal_emitter().to_owned();
                drop(interface);
                emit_changed(&emitter, &player).await;
            }
        }

        if let Ok(mut published) = self.published.lock() {
            published.insert(player.id.clone(), player);
        }
    }

    fn remove(&self, player_id: &str) {
        if let Ok(mut published) = self.published.lock() {
            published.remove(player_id);
        }
    }
}

async fn emit_changed(emitter: &SignalEmitter<'static>, player: &MediaPlayer) {
    let mut changed: HashMap<&str, Value<'_>> = HashMap::new();
    changed.insert(
        "PlaybackStatus",
        Value::from(playback_status(player.status)),
    );
    changed.insert("Metadata", Value::from(metadata_map(player)));
    let capabilities = player.capabilities.clone();
    changed.insert(
        "CanPlay",
        Value::from(capabilities.supports(mist_app_contract::MediaCommand::Play)),
    );
    changed.insert(
        "CanPause",
        Value::from(capabilities.supports(mist_app_contract::MediaCommand::Pause)),
    );
    let _ = zbus::fdo::Properties::properties_changed(
        emitter,
        "org.mpris.MediaPlayer2.Player".try_into().unwrap(),
        changed,
        std::borrow::Cow::Borrowed(&[]),
    )
    .await;
}

/// Shared mutable state for one exported player.
#[derive(Debug)]
struct SharedPlayer {
    state: Arc<Mutex<MediaPlayer>>,
    media: Arc<dyn Media>,
}

impl SharedPlayer {
    fn snapshot(&self) -> Option<MediaPlayer> {
        self.state.lock().ok().map(|player| player.clone())
    }

    fn dispatch(&self, command: MediaCommand) -> zbus::fdo::Result<()> {
        let player_id = self
            .snapshot()
            .map(|player| player.id)
            .ok_or_else(|| zbus::fdo::Error::Failed("media state unavailable".to_owned()))?;
        let media = Arc::clone(&self.media);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
        runtime
            .block_on(media.command(player_id, command))
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }
}

impl std::ops::Deref for SharedPlayer {
    type Target = Arc<Mutex<MediaPlayer>>;

    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

/// `org.mpris.MediaPlayer2` root interface.
#[derive(Debug)]
pub struct MistPlayerRoot {
    state: Arc<Mutex<MediaPlayer>>,
}

#[interface(name = "org.mpris.MediaPlayer2")]
#[allow(clippy::unused_self, reason = "D-Bus interface methods require &self")]
impl MistPlayerRoot {
    #[zbus(property)]
    fn identity(&self) -> String {
        self.state.lock().map_or_else(
            |_| "Mist".to_owned(),
            |player| format!("Mist · {}", player.device.display_name),
        )
    }

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn can_raise(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }

    fn raise(&self) {}
    fn quit(&self) {}
}

/// `org.mpris.MediaPlayer2.Player` transport interface.
#[derive(Debug)]
pub struct MistPlayerControls {
    state: Arc<Mutex<MediaPlayer>>,
    media: Arc<dyn Media>,
}

impl MistPlayerControls {
    fn replace(&self, player: MediaPlayer) {
        if let Ok(mut current) = self.state.lock() {
            *current = player;
        }
    }

    fn shared(&self) -> SharedPlayer {
        SharedPlayer {
            state: Arc::clone(&self.state),
            media: Arc::clone(&self.media),
        }
    }
}

#[interface(name = "org.mpris.MediaPlayer2.Player")]
#[allow(
    clippy::unused_self,
    clippy::used_underscore_binding,
    reason = "D-Bus interface methods require &self and unused parameters"
)]
impl MistPlayerControls {
    fn play(&self) -> zbus::fdo::Result<()> {
        self.shared().dispatch(MediaCommand::Play)
    }

    fn pause(&self) -> zbus::fdo::Result<()> {
        self.shared().dispatch(MediaCommand::Pause)
    }

    fn play_pause(&self) -> zbus::fdo::Result<()> {
        self.shared().dispatch(MediaCommand::PlayPause)
    }

    fn stop(&self) -> zbus::fdo::Result<()> {
        self.shared().dispatch(MediaCommand::Stop)
    }

    fn next(&self) -> zbus::fdo::Result<()> {
        self.shared().dispatch(MediaCommand::Next)
    }

    fn previous(&self) -> zbus::fdo::Result<()> {
        self.shared().dispatch(MediaCommand::Previous)
    }

    #[zbus(property)]
    fn playback_status(&self) -> String {
        self.shared()
            .snapshot()
            .map_or("Stopped", |player| playback_status(player.status))
            .to_owned()
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        self.shared().snapshot().is_some_and(|player| {
            player
                .capabilities
                .supports(mist_app_contract::MediaCommand::Play)
        })
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        self.shared().snapshot().is_some_and(|player| {
            player
                .capabilities
                .supports(mist_app_contract::MediaCommand::Pause)
        })
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        self.shared().snapshot().is_some_and(|player| {
            player
                .capabilities
                .supports(mist_app_contract::MediaCommand::Next)
        })
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        self.shared().snapshot().is_some_and(|player| {
            player
                .capabilities
                .supports(mist_app_contract::MediaCommand::Previous)
        })
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        self.shared()
            .snapshot()
            .map_or_else(HashMap::new, |player| metadata_map(&player))
    }

    fn seek(&self, offset: i64) {
        let _ = offset;
    }

    fn set_position(&self, track_id: ObjectPath<'_>, position: i64) {
        let _ = (track_id, position);
    }

    #[zbus(property)]
    fn position(&self) -> i64 {
        self.shared()
            .snapshot()
            .and_then(|player| player.position_ms)
            .and_then(|position| i64::try_from(position.saturating_mul(1000)).ok())
            .unwrap_or(0)
    }
}

fn playback_status(status: mist_app_contract::PlaybackStatus) -> &'static str {
    match status {
        mist_app_contract::PlaybackStatus::Playing => "Playing",
        mist_app_contract::PlaybackStatus::Paused => "Paused",
        mist_app_contract::PlaybackStatus::Stopped => "Stopped",
    }
}

fn metadata_map(player: &MediaPlayer) -> HashMap<String, OwnedValue> {
    let mut metadata = HashMap::new();
    insert_string(&mut metadata, "xesam:title", &player.metadata.title);
    insert_string(&mut metadata, "xesam:album", &player.metadata.album);
    if !player.metadata.artist.is_empty() {
        let artists = player
            .metadata
            .artist
            .split(", ")
            .map(ToOwned::to_owned)
            .collect::<Vec<String>>();
        if let Ok(owned) = OwnedValue::try_from(Value::from(artists)) {
            metadata.insert("xesam:artist".to_owned(), owned);
        }
    }
    insert_string(&mut metadata, "mpris:artUrl", &player.metadata.art_url);
    if let Some(duration) = player.metadata.duration_ms
        && let Ok(owned) = OwnedValue::try_from(Value::from(duration.saturating_mul(1000)))
    {
        metadata.insert("mpris:length".to_owned(), owned);
    }
    metadata
}

fn insert_string(metadata: &mut HashMap<String, OwnedValue>, key: &str, value: &str) {
    if value.is_empty() {
        return;
    }
    if let Ok(owned) = OwnedValue::try_from(Value::from(value.to_owned())) {
        metadata.insert(key.to_owned(), owned);
    }
}

fn sanitize(player_id: &str) -> String {
    player_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect()
}
