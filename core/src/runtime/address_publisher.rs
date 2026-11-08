use std::sync::atomic::{AtomicU64, Ordering};

use iroh::TransportAddr;
use iroh::address_lookup::{AddressLookup, EndpointData};
use tokio::sync::watch;

#[derive(Clone, Debug)]
pub(crate) struct PublishedAddresses {
    pub(crate) revision: u64,
    pub(crate) addresses: Vec<TransportAddr>,
}

#[derive(Debug)]
pub(crate) struct LocalAddressPublisher {
    revision: AtomicU64,
    updates: watch::Sender<Option<PublishedAddresses>>,
}

impl LocalAddressPublisher {
    pub(crate) fn new() -> (Self, watch::Receiver<Option<PublishedAddresses>>) {
        let (updates, receiver) = watch::channel(None);
        (
            Self {
                revision: AtomicU64::new(0),
                updates,
            },
            receiver,
        )
    }
}

impl AddressLookup for LocalAddressPublisher {
    fn publish(&self, data: &EndpointData) {
        let revision = self.revision.fetch_add(1, Ordering::Relaxed) + 1;
        self.updates.send_replace(Some(PublishedAddresses {
            revision,
            addresses: data.addrs().cloned().collect(),
        }));
    }
}
