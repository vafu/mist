use std::sync::Arc;

use iroh::EndpointId;
use tokio_util::sync::CancellationToken;

use super::{DeviceError, DeviceStore, Devices, Peer};
use crate::Pairing;
use crate::runtime::Connections;

pub(super) fn validate_peer(peer: &Peer) -> Result<(), DeviceError> {
    let id = peer
        .endpoint_id
        .parse::<EndpointId>()
        .map_err(|error| DeviceError::InvalidData {
            reason: format!("invalid peer identity: {error}"),
        })?;
    if id.to_string() != peer.endpoint_id {
        return Err(DeviceError::InvalidData {
            reason: "peer identity must use canonical encoding".to_owned(),
        });
    }
    if peer.device.display_name.trim().is_empty() || peer.device.display_name.chars().count() > 128
    {
        return Err(DeviceError::InvalidData {
            reason: "invalid peer display name".to_owned(),
        });
    }
    Ok(())
}

#[derive(Debug)]
pub(crate) struct DeviceService {
    pub(crate) store: Arc<dyn DeviceStore>,
    pub(crate) connections: Arc<dyn Connections>,
    pub(crate) pairing: Arc<dyn Pairing>,
    pub(crate) shutdown: CancellationToken,
}

impl Devices for DeviceService {
    fn peers(&self) -> Result<Vec<Peer>, DeviceError> {
        if self.shutdown.is_cancelled() {
            return Err(DeviceError::Closed);
        }
        self.store.peers()
    }
    fn forget(&self, endpoint_id: String) -> Result<(), DeviceError> {
        if self.shutdown.is_cancelled() {
            return Err(DeviceError::Closed);
        }
        let id = endpoint_id
            .parse::<EndpointId>()
            .map_err(|error| DeviceError::InvalidData {
                reason: error.to_string(),
            })?;
        self.pairing.cancel();
        self.connections.forget(id)
    }
}
