use std::collections::HashMap;
use std::sync::{PoisonError, RwLock};

use iroh::endpoint::Connection;
use iroh::{EndpointId, TransportAddr};
use tokio::sync::broadcast;

use crate::storage::StoredDevice;
use crate::{
    ConnectionTransport, DeviceError, DeviceEvent, DeviceEventKind, DeviceInfo, DeviceStatus,
    ManagedDevice,
};

#[derive(Debug)]
struct PeerEntry {
    device: DeviceInfo,
    connection: Option<Connection>,
    selected: bool,
    generation: u64,
    connecting: bool,
}

#[derive(Debug)]
pub(crate) struct DeviceRegistry {
    peers: RwLock<HashMap<EndpointId, PeerEntry>>,
    events: broadcast::Sender<DeviceEvent>,
}

impl DeviceRegistry {
    pub(crate) fn new(stored: Vec<StoredDevice>) -> Self {
        let (events, _) = broadcast::channel(128);
        let peers = stored
            .into_iter()
            .map(|stored| {
                (
                    stored.endpoint_id,
                    PeerEntry {
                        device: stored.device,
                        connection: None,
                        selected: stored.selected,
                        generation: 0,
                        connecting: false,
                    },
                )
            })
            .collect();
        Self {
            peers: RwLock::new(peers),
            events,
        }
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<DeviceEvent> {
        self.events.subscribe()
    }

    pub(crate) fn devices(&self) -> Vec<ManagedDevice> {
        let peers = self.peers.read().unwrap_or_else(PoisonError::into_inner);
        let mut devices = peers
            .iter()
            .map(|(endpoint_id, entry)| snapshot(*endpoint_id, entry))
            .collect::<Vec<_>>();
        devices.sort_by(|left, right| {
            left.device
                .display_name
                .cmp(&right.device.display_name)
                .then_with(|| left.endpoint_id.cmp(&right.endpoint_id))
        });
        devices
    }

    pub(crate) fn device(&self, endpoint_id: EndpointId) -> Option<ManagedDevice> {
        self.peers
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&endpoint_id)
            .map(|entry| snapshot(endpoint_id, entry))
    }

    pub(crate) fn contains(&self, endpoint_id: EndpointId) -> bool {
        self.peers
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(&endpoint_id)
    }

    pub(crate) fn device_info(&self, endpoint_id: EndpointId) -> Option<DeviceInfo> {
        self.peers
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&endpoint_id)
            .map(|entry| entry.device.clone())
    }

    pub(crate) fn connection(&self, endpoint_id: EndpointId) -> Option<Connection> {
        self.peers
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&endpoint_id)
            .and_then(|entry| entry.connection.clone())
    }

    pub(crate) fn selected_connections(&self) -> Vec<(EndpointId, Option<Connection>)> {
        self.peers
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(_, entry)| entry.selected)
            .map(|(endpoint_id, entry)| (*endpoint_id, entry.connection.clone()))
            .collect()
    }

    pub(crate) fn install(
        &self,
        endpoint_id: EndpointId,
        device: DeviceInfo,
        connection: Connection,
    ) -> u64 {
        let (kind, snapshot, previous) = {
            let mut peers = self.peers.write().unwrap_or_else(PoisonError::into_inner);
            let existed = peers.contains_key(&endpoint_id);
            let entry = peers.entry(endpoint_id).or_insert_with(|| PeerEntry {
                device: device.clone(),
                connection: None,
                selected: true,
                generation: 0,
                connecting: false,
            });
            entry.generation = entry.generation.wrapping_add(1);
            entry.device = device;
            entry.connecting = false;
            let previous = entry.connection.replace(connection);
            let generation = entry.generation;
            (
                if existed {
                    DeviceEventKind::Changed
                } else {
                    DeviceEventKind::Added
                },
                snapshot(endpoint_id, entry),
                (generation, previous),
            )
        };
        if let Some(previous) = previous.1 {
            previous.close(0_u32.into(), b"connection replaced");
        }
        let _ = self.events.send(DeviceEvent {
            kind,
            device: snapshot,
        });
        previous.0
    }

    pub(crate) fn set_connecting(&self, endpoint_id: EndpointId) -> Result<(), DeviceError> {
        let snapshot = {
            let mut peers = self.peers.write().unwrap_or_else(PoisonError::into_inner);
            let entry = peers
                .get_mut(&endpoint_id)
                .ok_or_else(|| DeviceError::UnknownDevice {
                    endpoint_id: endpoint_id.to_string(),
                })?;
            entry.connecting = true;
            snapshot(endpoint_id, entry)
        };
        let _ = self.events.send(DeviceEvent {
            kind: DeviceEventKind::Changed,
            device: snapshot,
        });
        Ok(())
    }

    pub(crate) fn mark_disconnected(&self, endpoint_id: EndpointId, generation: u64) {
        let snapshot = {
            let mut peers = self.peers.write().unwrap_or_else(PoisonError::into_inner);
            let Some(entry) = peers.get_mut(&endpoint_id) else {
                return;
            };
            if entry.generation != generation {
                return;
            }
            entry.connection = None;
            entry.connecting = false;
            snapshot(endpoint_id, entry)
        };
        let _ = self.events.send(DeviceEvent {
            kind: DeviceEventKind::Changed,
            device: snapshot,
        });
    }

    pub(crate) fn clear_connecting(&self, endpoint_id: EndpointId) {
        let snapshot = {
            let mut peers = self.peers.write().unwrap_or_else(PoisonError::into_inner);
            let Some(entry) = peers.get_mut(&endpoint_id) else {
                return;
            };
            entry.connecting = false;
            snapshot(endpoint_id, entry)
        };
        let _ = self.events.send(DeviceEvent {
            kind: DeviceEventKind::Changed,
            device: snapshot,
        });
    }

    pub(crate) fn notify_changed(&self, endpoint_id: EndpointId, generation: u64) {
        let snapshot = {
            let peers = self.peers.read().unwrap_or_else(PoisonError::into_inner);
            let Some(entry) = peers.get(&endpoint_id) else {
                return;
            };
            if entry.generation != generation {
                return;
            }
            snapshot(endpoint_id, entry)
        };
        let _ = self.events.send(DeviceEvent {
            kind: DeviceEventKind::Changed,
            device: snapshot,
        });
    }

    pub(crate) fn select(
        &self,
        endpoint_id: EndpointId,
        selected: bool,
    ) -> Result<(), DeviceError> {
        let snapshot = {
            let mut peers = self.peers.write().unwrap_or_else(PoisonError::into_inner);
            let entry = peers
                .get_mut(&endpoint_id)
                .ok_or_else(|| DeviceError::UnknownDevice {
                    endpoint_id: endpoint_id.to_string(),
                })?;
            entry.selected = selected;
            snapshot(endpoint_id, entry)
        };
        let _ = self.events.send(DeviceEvent {
            kind: DeviceEventKind::Changed,
            device: snapshot,
        });
        Ok(())
    }

    pub(crate) fn disconnect(&self, endpoint_id: EndpointId) -> Result<(), DeviceError> {
        let (connection, snapshot) = {
            let mut peers = self.peers.write().unwrap_or_else(PoisonError::into_inner);
            let entry = peers
                .get_mut(&endpoint_id)
                .ok_or_else(|| DeviceError::UnknownDevice {
                    endpoint_id: endpoint_id.to_string(),
                })?;
            entry.generation = entry.generation.wrapping_add(1);
            entry.connecting = false;
            let connection = entry.connection.take();
            (connection, snapshot(endpoint_id, entry))
        };
        let connection = connection.ok_or_else(|| DeviceError::Disconnected {
            endpoint_id: endpoint_id.to_string(),
        })?;
        connection.close(0_u32.into(), b"disconnected by user");
        let _ = self.events.send(DeviceEvent {
            kind: DeviceEventKind::Changed,
            device: snapshot,
        });
        Ok(())
    }

    pub(crate) fn forget(&self, endpoint_id: EndpointId) -> Result<(), DeviceError> {
        let (connection, snapshot) = {
            let mut peers = self.peers.write().unwrap_or_else(PoisonError::into_inner);
            let entry = peers
                .remove(&endpoint_id)
                .ok_or_else(|| DeviceError::UnknownDevice {
                    endpoint_id: endpoint_id.to_string(),
                })?;
            let snapshot = snapshot(endpoint_id, &entry);
            (entry.connection, snapshot)
        };
        if let Some(connection) = connection {
            connection.close(0_u32.into(), b"device forgotten");
        }
        let _ = self.events.send(DeviceEvent {
            kind: DeviceEventKind::Removed,
            device: snapshot,
        });
        Ok(())
    }

    pub(crate) fn close_all(&self) {
        let connections = {
            let mut peers = self.peers.write().unwrap_or_else(PoisonError::into_inner);
            peers
                .values_mut()
                .filter_map(|entry| entry.connection.take())
                .collect::<Vec<_>>()
        };
        for connection in connections {
            connection.close(0_u32.into(), b"Mist runtime closed");
        }
    }
}

fn snapshot(endpoint_id: EndpointId, entry: &PeerEntry) -> ManagedDevice {
    let status = if entry.connecting {
        DeviceStatus::Connecting
    } else if entry.connection.is_some() {
        DeviceStatus::Connected
    } else {
        DeviceStatus::Disconnected
    };
    let transport = entry
        .connection
        .as_ref()
        .map_or(ConnectionTransport::Unknown, |connection| {
            let paths = connection.paths();
            paths.iter().find(iroh::endpoint::Path::is_selected).map_or(
                ConnectionTransport::Unknown,
                |path| match path.remote_addr() {
                    TransportAddr::Ip(_) => ConnectionTransport::Direct,
                    TransportAddr::Relay(_) => ConnectionTransport::Relay,
                    TransportAddr::Custom(_) => ConnectionTransport::Custom,
                    _ => ConnectionTransport::Unknown,
                },
            )
        });
    ManagedDevice {
        endpoint_id: endpoint_id.to_string(),
        device: entry.device.clone(),
        status,
        transport,
        selected: entry.selected,
    }
}
