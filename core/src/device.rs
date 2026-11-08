//! Device-manager implementation objects.

use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use iroh::EndpointId;
use mist_app_contract::{
    DeviceError, DeviceEvent, DeviceEventError, DeviceEventStream, DeviceManager, ManagedDevice,
};
use tokio::sync::{Mutex, broadcast};

use crate::runtime::Session;

#[derive(Debug)]
pub(crate) struct RuntimeDeviceEventStream {
    receiver: Arc<Mutex<broadcast::Receiver<DeviceEvent>>>,
}

impl RuntimeDeviceEventStream {
    pub(crate) fn new(receiver: broadcast::Receiver<DeviceEvent>) -> Arc<Self> {
        Arc::new(Self {
            receiver: Arc::new(Mutex::new(receiver)),
        })
    }
}

#[async_trait]
impl DeviceEventStream for RuntimeDeviceEventStream {
    async fn next(&self) -> Result<DeviceEvent, DeviceEventError> {
        self.receiver
            .lock()
            .await
            .recv()
            .await
            .map_err(|error| match error {
                broadcast::error::RecvError::Closed => DeviceEventError::Closed,
                broadcast::error::RecvError::Lagged(count) => DeviceEventError::Lagged { count },
            })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RuntimeDeviceManager {
    session: Arc<Session>,
}

impl RuntimeDeviceManager {
    pub(crate) fn new(session: Arc<Session>) -> Self {
        Self { session }
    }

    fn endpoint_id(raw: &str) -> Result<EndpointId, DeviceError> {
        EndpointId::from_str(raw).map_err(|error| DeviceError::InvalidEndpointId {
            reason: error.to_string(),
        })
    }
}

#[async_trait]
impl DeviceManager for RuntimeDeviceManager {
    fn list(&self) -> Vec<ManagedDevice> {
        self.session.devices()
    }

    fn get(&self, endpoint_id: String) -> Result<ManagedDevice, DeviceError> {
        let id = Self::endpoint_id(&endpoint_id)?;
        self.session
            .device(id)
            .ok_or(DeviceError::UnknownDevice { endpoint_id })
    }

    fn subscribe(&self) -> Arc<dyn DeviceEventStream> {
        RuntimeDeviceEventStream::new(self.session.subscribe_devices())
    }

    fn select(&self, endpoint_id: String, selected: bool) -> Result<(), DeviceError> {
        self.session
            .select_device(Self::endpoint_id(&endpoint_id)?, selected)
    }

    async fn connect(&self, endpoint_id: String) -> Result<(), DeviceError> {
        let endpoint_id = Self::endpoint_id(&endpoint_id)?;
        let session = Arc::clone(&self.session);
        let executor = session.executor().clone();
        executor
            .spawn(async move { session.connect_device(endpoint_id).await })
            .await
            .map_err(|error| DeviceError::Executor {
                reason: error.to_string(),
            })?
    }

    fn disconnect(&self, endpoint_id: String) -> Result<(), DeviceError> {
        self.session
            .disconnect_device(Self::endpoint_id(&endpoint_id)?)
    }

    fn forget(&self, endpoint_id: String) -> Result<(), DeviceError> {
        self.session.forget_device(Self::endpoint_id(&endpoint_id)?)
    }
}
