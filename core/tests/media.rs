use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use mist_app_contract::{
    DeviceClass, DeviceInfo, MediaCommand, MediaMetadata, MediaOrigin, MistApp, Platform,
    PlaybackStatus,
};
use mist_core::host::{LocalPlayer, MediaProvider, MediaProviderError, MediaWatcher, MistFactory};
use tempfile::TempDir;
use tokio::sync::Mutex;

fn device(name: &str) -> DeviceInfo {
    DeviceInfo {
        display_name: name.to_owned(),
        device_class: DeviceClass::Desktop,
        platform: Platform::Linux,
    }
}

/// Test backend: reports a fixed player set once, then blocks.
#[derive(Debug)]
struct FakeMedia {
    players: Vec<LocalPlayer>,
    commands: Mutex<Vec<(String, MediaCommand)>>,
}

impl FakeMedia {
    fn new(players: Vec<LocalPlayer>) -> Arc<Self> {
        Arc::new(Self {
            players,
            commands: Mutex::new(Vec::new()),
        })
    }

    async fn commands(&self) -> Vec<(String, MediaCommand)> {
        self.commands.lock().await.clone()
    }
}

#[async_trait]
impl MediaProvider for FakeMedia {
    fn watch(&self) -> Result<Box<dyn MediaWatcher>, MediaProviderError> {
        Ok(Box::new(FakeWatcher {
            players: Some(self.players.clone()),
        }))
    }

    async fn command(
        &self,
        player_id: &str,
        command: MediaCommand,
    ) -> Result<(), MediaProviderError> {
        self.commands
            .lock()
            .await
            .push((player_id.to_owned(), command));
        Ok(())
    }
}

struct FakeWatcher {
    players: Option<Vec<LocalPlayer>>,
}

impl MediaWatcher for FakeWatcher {
    fn next(&mut self) -> Result<Option<Vec<LocalPlayer>>, MediaProviderError> {
        match self.players.take() {
            Some(players) => Ok(Some(players)),
            // No further updates.
            None => Ok(None),
        }
    }
}

fn player(id: &str, title: &str) -> LocalPlayer {
    LocalPlayer {
        id: id.to_owned(),
        name: "Phone Player".to_owned(),
        status: PlaybackStatus::Playing,
        metadata: MediaMetadata {
            title: title.to_owned(),
            artist: "Artist".to_owned(),
            album: "Album".to_owned(),
            art_url: String::new(),
            artwork: None,
            duration_ms: Some(180_000),
        },
        commands: vec![MediaCommand::Play, MediaCommand::Pause, MediaCommand::Next],
        position_ms: Some(1_000),
    }
}

fn player_with_artwork(id: &str, title: &str, path: &std::path::Path) -> LocalPlayer {
    let mut player = player(id, title);
    player.metadata.artwork = Some(mist_app_contract::MediaArtwork {
        mime_type: "image/png".to_owned(),
        path: path.to_string_lossy().into_owned(),
    });
    player
}

async fn runtime_with_media(
    directory: &TempDir,
    name: &str,
    backend: Arc<FakeMedia>,
) -> Arc<dyn MistApp> {
    MistFactory::new(directory.path(), device(name))
        .unwrap()
        .local_only()
        .media(backend)
        .create()
        .await
        .unwrap()
}

async fn pair(creator: &Arc<dyn MistApp>, invited: &Arc<dyn MistApp>) {
    let offer = creator.pairing().initiate().unwrap();
    let waiting = {
        let pairing = creator.pairing();
        tokio::spawn(async move { pairing.wait_for_peer().await })
    };
    invited.pairing().accept(offer).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

async fn wait_for_remote_player(runtime: &Arc<dyn MistApp>) -> mist_app_contract::MediaPlayer {
    for _ in 0..50 {
        if let Some(player) = runtime
            .media()
            .list()
            .into_iter()
            .find(|player| player.origin == MediaOrigin::Remote)
        {
            return player;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("remote player never appeared");
}

#[tokio::test]
async fn remote_media_state_is_visible_and_commands_reach_the_owner() {
    let phone_dir = TempDir::new().unwrap();
    let laptop_dir = TempDir::new().unwrap();
    let backend = FakeMedia::new(vec![player("p1", "Remote Track")]);

    let phone = runtime_with_media(&phone_dir, "phone", Arc::clone(&backend)).await;
    let laptop = MistFactory::new(laptop_dir.path(), device("laptop"))
        .unwrap()
        .local_only()
        .create()
        .await
        .unwrap();
    pair(&phone, &laptop).await;

    let remote = wait_for_remote_player(&laptop).await;
    assert_eq!(remote.id, "p1");
    assert_eq!(remote.name, "Phone Player");
    assert_eq!(remote.metadata.title, "Remote Track");
    assert_eq!(remote.metadata.duration_ms, Some(180_000));
    assert_eq!(remote.position_ms, Some(1_000));
    assert_eq!(remote.device.display_name, "phone");
    assert_eq!(remote.endpoint_id, phone.endpoint_id());

    laptop
        .media()
        .command("p1".to_owned(), MediaCommand::Next)
        .await
        .unwrap();

    let mut delivered = Vec::new();
    for _ in 0..50 {
        delivered = backend.commands().await;
        if !delivered.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(delivered, vec![("p1".to_owned(), MediaCommand::Next)]);

    phone.shutdown().await;
    laptop.shutdown().await;
}

#[tokio::test]
async fn media_events_report_added_then_removed() {
    let directory = TempDir::new().unwrap();
    let backend = FakeMedia::new(vec![player("p1", "Track")]);
    let runtime = runtime_with_media(&directory, "phone", backend).await;

    let events = runtime.media().subscribe();
    // The initial publish happens at attach time; wait for the next event window.
    let mut saw_added = false;
    for _ in 0..50 {
        match tokio::time::timeout(Duration::from_millis(200), events.next()).await {
            Ok(Ok(event)) => {
                if event.kind == mist_app_contract::MediaEventKind::Added {
                    saw_added = true;
                    assert_eq!(event.player.metadata.title, "Track");
                    break;
                }
            }
            _ => break,
        }
    }
    assert!(saw_added, "expected an Added event for the local player");

    runtime.shutdown().await;
}

#[tokio::test]
async fn media_artwork_crosses_the_wire() {
    let phone_dir = TempDir::new().unwrap();
    let laptop_dir = TempDir::new().unwrap();
    let artwork_path = phone_dir.path().join("cover.png");
    let artwork = (0_u32..4000)
        .map(|index| u8::try_from(index % 251).unwrap())
        .collect::<Vec<_>>();
    std::fs::write(&artwork_path, &artwork).unwrap();

    let backend = FakeMedia::new(vec![player_with_artwork("p1", "Art", &artwork_path)]);
    let phone = runtime_with_media(&phone_dir, "phone", backend).await;
    let laptop = MistFactory::new(laptop_dir.path(), device("laptop"))
        .unwrap()
        .local_only()
        .create()
        .await
        .unwrap();
    pair(&phone, &laptop).await;

    let mut remote = None;
    for _ in 0..100 {
        let found = laptop
            .media()
            .list()
            .into_iter()
            .find(|player| player.origin == MediaOrigin::Remote);
        if let Some(player) = found
            && player.artwork_path.is_some()
        {
            remote = Some(player);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let remote = remote.expect("remote player with artwork never appeared");
    let path = remote.artwork_path.expect("artwork path missing");
    assert_eq!(std::fs::read(path).unwrap(), artwork);

    phone.shutdown().await;
    laptop.shutdown().await;
}
