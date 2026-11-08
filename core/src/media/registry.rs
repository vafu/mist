//! Media player state for this device and every connected peer.

use std::collections::HashMap;
use std::sync::{PoisonError, RwLock};

use mist_app_contract::{DeviceInfo, MediaEvent, MediaEventKind, MediaOrigin, MediaPlayer};
use tokio::sync::broadcast;

use super::LocalPlayer;

/// Player bookkeeping keyed by origin and player identity.
#[derive(Debug)]
pub(crate) struct MediaRegistry {
    local_device: DeviceInfo,
    local: RwLock<Vec<MediaPlayer>>,
    remote: RwLock<HashMap<String, (DeviceInfo, Vec<MediaPlayer>)>>,
    events: broadcast::Sender<MediaEvent>,
}

impl MediaRegistry {
    pub(crate) fn new(local_device: DeviceInfo) -> Self {
        let (events, _) = broadcast::channel(128);
        Self {
            local_device,
            local: RwLock::new(Vec::new()),
            remote: RwLock::new(HashMap::new()),
            events,
        }
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<MediaEvent> {
        self.events.subscribe()
    }

    /// Returns local players followed by remote players, in stable order.
    pub(crate) fn list(&self) -> Vec<MediaPlayer> {
        let mut players = self
            .local
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let remote = self.remote.read().unwrap_or_else(PoisonError::into_inner);
        let mut remote_players = remote
            .values()
            .flat_map(|(_, players)| players.iter().cloned())
            .collect::<Vec<_>>();
        remote_players.sort_by(|left, right| {
            left.endpoint_id
                .cmp(&right.endpoint_id)
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.id.cmp(&right.id))
        });
        players.extend(remote_players);
        players
    }

    /// Replaces the local player set and emits diff events.
    pub(crate) fn set_local(&self, players: &[LocalPlayer]) {
        let next = players
            .iter()
            .map(|player| super::local_player(&self.local_device, player))
            .collect::<Vec<_>>();
        let previous = {
            let mut local = self.local.write().unwrap_or_else(PoisonError::into_inner);
            std::mem::replace(&mut *local, next.clone())
        };
        for event in diff_events(&previous, &next) {
            let _ = self.events.send(event);
        }
    }

    /// Replaces the player set reported by one peer.
    pub(crate) fn set_remote(
        &self,
        endpoint_id: &str,
        device: &DeviceInfo,
        players: &[MediaPlayer],
    ) {
        let previous = {
            let mut remote = self.remote.write().unwrap_or_else(PoisonError::into_inner);
            let previous = remote
                .get(endpoint_id)
                .map(|(_, players)| players.clone())
                .unwrap_or_default();
            if players.is_empty() {
                remote.remove(endpoint_id);
            } else {
                remote.insert(endpoint_id.to_owned(), (device.clone(), players.to_vec()));
            }
            previous
        };
        for event in diff_events(&previous, players) {
            let _ = self.events.send(event);
        }
    }

    /// Removes every player reported by a peer.
    pub(crate) fn remove_remote(&self, endpoint_id: &str) {
        let previous = {
            let mut remote = self.remote.write().unwrap_or_else(PoisonError::into_inner);
            remote
                .remove(endpoint_id)
                .map(|(_, players)| players)
                .unwrap_or_default()
        };
        for player in previous {
            let _ = self.events.send(MediaEvent {
                kind: MediaEventKind::Removed,
                player,
            });
        }
    }
}

fn diff_events(previous: &[MediaPlayer], next: &[MediaPlayer]) -> Vec<MediaEvent> {
    let mut events = Vec::new();
    for player in next {
        match previous.iter().find(|known| known.id == player.id) {
            Some(known) if known == player => {}
            Some(_) => events.push(MediaEvent {
                kind: MediaEventKind::Changed,
                player: player.clone(),
            }),
            None => events.push(MediaEvent {
                kind: MediaEventKind::Added,
                player: player.clone(),
            }),
        }
    }
    for player in previous {
        if !next.iter().any(|known| known.id == player.id) {
            events.push(MediaEvent {
                kind: MediaEventKind::Removed,
                player: player.clone(),
            });
        }
    }
    events
}

/// Builds a remote player snapshot from wire values.
pub(crate) fn remote_player(
    endpoint_id: &str,
    device: &DeviceInfo,
    player: &crate::protocol::MediaPlayerState,
) -> MediaPlayer {
    MediaPlayer {
        id: player.id.clone(),
        endpoint_id: endpoint_id.to_owned(),
        name: player.name.clone(),
        device: device.clone(),
        origin: MediaOrigin::Remote,
        status: player.status,
        metadata: player.metadata.clone(),
        capabilities: player.capabilities.clone(),
        position_ms: player.position_ms,
    }
}
