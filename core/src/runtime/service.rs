use std::sync::Arc;

use async_trait::async_trait;
use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointAddr, EndpointId};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::connections::IrohConnections;
use super::{Connections, MistBuilder, Runtime, RuntimeError, executor, run_owned};
use crate::device::DeviceService;
use crate::pairing::IrohPairing;
use crate::{DeviceInfo, DeviceStore, Devices, FileStore, Pairing};

impl From<crate::DeviceError> for RuntimeError {
    fn from(error: crate::DeviceError) -> Self {
        match error {
            crate::DeviceError::Closed => Self::Closed,
            other => Self::Storage {
                reason: other.to_string(),
            },
        }
    }
}

/// Shared handle to one running Mist instance.
///
/// Domain behavior is defined by [`Runtime`]. Generated bindings delegate to
/// that implementation on this same allocation, not another runtime or registry.
#[derive(Debug, uniffi::Object)]
pub struct MistRuntime {
    endpoint: Endpoint,
    connections: Arc<dyn Connections>,
    pairing: Arc<dyn Pairing>,
    devices: Arc<dyn Devices>,
    shutdown: CancellationToken,
    tasks: TaskTracker,
}

impl Drop for MistRuntime {
    fn drop(&mut self) {
        self.shutdown.cancel();
        self.pairing.cancel();
        self.connections.close();
        self.tasks.close();
        executor::close_dropped_instance(self.endpoint.clone(), self.tasks.clone());
    }
}

impl MistRuntime {
    pub(crate) fn from_endpoint(
        endpoint: Endpoint,
        device_info: DeviceInfo,
        store: Arc<dyn DeviceStore>,
    ) -> Arc<Self> {
        let shutdown = CancellationToken::new();
        let tasks = TaskTracker::new();
        let connections: Arc<dyn Connections> = Arc::new(IrohConnections::new(
            endpoint.clone(),
            device_info.clone(),
            shutdown.clone(),
            tasks.clone(),
            Arc::clone(&store),
        ));
        let pairing = Arc::new(IrohPairing::new(
            Arc::clone(&connections),
            device_info,
            shutdown.clone(),
            tasks.clone(),
        ));
        tasks.spawn(Arc::clone(&connections).run(pairing.clone()));
        let devices = Arc::new(DeviceService {
            store,
            connections: Arc::clone(&connections),
            pairing: pairing.clone(),
            shutdown: shutdown.clone(),
        });
        Arc::new(Self {
            endpoint,
            connections,
            pairing,
            devices,
            shutdown,
            tasks,
        })
    }
}

#[async_trait]
impl Runtime for MistRuntime {
    fn address(&self) -> EndpointAddr {
        self.connections.address()
    }

    fn connection(&self, peer: EndpointId) -> Option<Connection> {
        self.connections.get(peer)
    }

    async fn connect(&self, peer: EndpointAddr) -> Result<Connection, RuntimeError> {
        let connections = Arc::clone(&self.connections);
        run_owned(&self.tasks, &self.shutdown, async move {
            connections.connect(peer).await
        })
        .await
    }

    async fn reconnect(&self, endpoint_id: String) -> Result<(), RuntimeError> {
        if self.shutdown.is_cancelled() {
            return Err(RuntimeError::Closed);
        }
        let id = endpoint_id
            .parse::<EndpointId>()
            .map_err(|error| RuntimeError::Connect {
                reason: format!("invalid endpoint identity: {error}"),
            })?;
        self.connect(id.into()).await.map(|_| ())
    }

    fn pairing(&self) -> Arc<dyn Pairing> {
        Arc::clone(&self.pairing)
    }

    fn devices(&self) -> Arc<dyn Devices> {
        Arc::clone(&self.devices)
    }

    fn is_connected(&self) -> bool {
        self.connections.is_connected()
    }

    async fn shutdown(&self) -> Result<(), RuntimeError> {
        self.shutdown.cancel();
        self.pairing.cancel();
        self.connections.close();
        self.tasks.close();
        let tasks = self.tasks.clone();
        let endpoint = self.endpoint.clone();
        executor::run(async move {
            endpoint.close().await;
            tasks.wait().await;
        })
        .await
    }
}

#[uniffi::export]
impl MistRuntime {
    /// Reconnects a saved peer using the core's configured address lookup.
    ///
    /// # Errors
    /// Returns identity, trust, transport, storage or closed-runtime failures.
    pub async fn reconnect(&self, endpoint_id: String) -> Result<(), RuntimeError> {
        Runtime::reconnect(self, endpoint_id).await
    }
    /// Returns saved-device operations from the shared core.
    #[must_use]
    pub fn devices(&self) -> Arc<dyn Devices> {
        Runtime::devices(self)
    }

    /// Returns this instance's pairing service.
    #[must_use]
    pub fn pairing(&self) -> Arc<dyn Pairing> {
        Runtime::pairing(self)
    }

    /// Returns the local Iroh endpoint identity.
    #[must_use]
    pub fn endpoint_id(&self) -> String {
        self.address().id.to_string()
    }

    /// Reports whether any registered peer connection is still open.
    #[must_use]
    pub fn is_connected(&self) -> bool {
        Runtime::is_connected(self)
    }

    /// Closes this instance's connections and endpoint.
    ///
    /// # Errors
    /// Returns an execution failure if shutdown could not complete.
    pub async fn shutdown(&self) -> Result<(), RuntimeError> {
        Runtime::shutdown(self).await
    }
}

/// Creates an instance with the default production endpoint configuration.
///
/// # Errors
/// Returns an executor or endpoint bind failure.
#[uniffi::export]
pub async fn create_mist_runtime(
    device_info: DeviceInfo,
    storage_directory: String,
) -> Result<Arc<MistRuntime>, RuntimeError> {
    executor::run(async move {
        let store = FileStore::open(storage_directory).map_err(|error| RuntimeError::Storage {
            reason: error.to_string(),
        })?;
        MistBuilder::new(device_info)
            .store(Arc::new(store))
            .bind()
            .await
    })
    .await?
}
