use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh::{EndpointId, SecretKey};
use mist_core::{
    DeviceError, DeviceInfo, DeviceStore, FileStore, MemoryStore, MistBuilder, MistRuntime,
    PairingError, Peer, Platform, Runtime, RuntimeError,
};

fn info(name: &str) -> DeviceInfo {
    DeviceInfo {
        display_name: name.to_owned(),
        platform: Platform::Linux,
    }
}

async fn pair(first: &MistRuntime, second: &MistRuntime) {
    let offering = first.pairing();
    let joining = second.pairing();
    let offer = offering.initiate().unwrap();
    let (a, b) = tokio::join!(offering.wait_for_peer(), joining.accept(offer));
    a.unwrap();
    b.unwrap();
}

#[tokio::test]
async fn identities_and_trust_survive_restart_and_either_peer_can_reconnect() {
    let a_dir = tempfile::tempdir().unwrap();
    let b_dir = tempfile::tempdir().unwrap();
    let a = MistBuilder::new(info("a"))
        .store(Arc::new(FileStore::open(a_dir.path()).unwrap()))
        .local_only()
        .bind()
        .await
        .unwrap();
    let b = MistBuilder::new(info("b"))
        .store(Arc::new(FileStore::open(b_dir.path()).unwrap()))
        .local_only()
        .bind()
        .await
        .unwrap();
    let a_id = a.address().id;
    let b_id = b.address().id;
    assert!(matches!(
        a.connect(b.address()).await,
        Err(RuntimeError::Untrusted { .. })
    ));
    for _ in 0..8 {
        pair(&a, &b).await;
    }
    assert_eq!(a.devices().peers().unwrap().len(), 1);
    assert_eq!(b.devices().peers().unwrap().len(), 1);
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
    drop(a);
    drop(b);
    let a = MistBuilder::new(info("a"))
        .store(Arc::new(FileStore::open(a_dir.path()).unwrap()))
        .local_only()
        .bind()
        .await
        .unwrap();
    let b = MistBuilder::new(info("b"))
        .store(Arc::new(FileStore::open(b_dir.path()).unwrap()))
        .local_only()
        .bind()
        .await
        .unwrap();
    assert_eq!(a.address().id, a_id);
    assert_eq!(b.address().id, b_id);
    assert!(!a.is_connected());
    assert_eq!(
        a.devices().peers().unwrap()[0].endpoint_id,
        b_id.to_string()
    );
    b.connect(a.address()).await.unwrap();
    let remote = a.connection(b_id).unwrap();
    b.connection(a_id)
        .unwrap()
        .close(0_u32.into(), b"reverse initiation");
    remote.closed().await;
    a.connect(b.address()).await.unwrap();
    let remote = b.connection(a_id).unwrap();
    a.devices().forget(b_id.to_string()).unwrap();
    remote.closed().await;
    assert!(a.devices().peers().unwrap().is_empty());
    assert_eq!(b.devices().peers().unwrap().len(), 1);
    assert!(matches!(
        a.connect(b.address()).await,
        Err(RuntimeError::Untrusted { .. })
    ));
    assert!(b.connect(a.address()).await.is_err());
    pair(&a, &b).await;
    assert_eq!(a.devices().peers().unwrap().len(), 1);
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
}

#[derive(Debug)]
struct FailingStore {
    memory: MemoryStore,
    fail_save: AtomicBool,
    fail_remove: AtomicBool,
}
impl DeviceStore for FailingStore {
    fn identity(&self) -> Result<SecretKey, DeviceError> {
        self.memory.identity()
    }
    fn peers(&self) -> Result<Vec<Peer>, DeviceError> {
        self.memory.peers()
    }
    fn get(&self, id: EndpointId) -> Result<Option<Peer>, DeviceError> {
        self.memory.get(id)
    }
    fn save(&self, peer: Peer) -> Result<(), DeviceError> {
        if self.fail_save.load(Ordering::SeqCst) {
            return Err(DeviceError::Io {
                reason: "injected save failure".to_owned(),
            });
        }
        self.memory.save(peer)
    }
    fn remove(&self, id: EndpointId) -> Result<(), DeviceError> {
        if self.fail_remove.load(Ordering::SeqCst) {
            return Err(DeviceError::Io {
                reason: "injected remove failure".to_owned(),
            });
        }
        self.memory.remove(id)
    }
}

#[tokio::test]
async fn persistence_failure_is_not_reported_as_pairing_or_forget_success() {
    let store = Arc::new(FailingStore {
        memory: MemoryStore::new(SecretKey::from_bytes(&[10; 32])),
        fail_save: AtomicBool::new(true),
        fail_remove: AtomicBool::new(false),
    });
    let a = MistBuilder::new(info("a"))
        .store(store.clone())
        .local_only()
        .bind()
        .await
        .unwrap();
    let b = MistBuilder::new(info("b"))
        .local_only()
        .bind()
        .await
        .unwrap();
    let offering = a.pairing();
    let joining = b.pairing();
    let offer = offering.initiate().unwrap();
    let (a_result, _) = tokio::join!(offering.wait_for_peer(), joining.accept(offer));
    assert!(matches!(a_result, Err(PairingError::Storage { .. })));
    assert!(a.devices().peers().unwrap().is_empty());
    assert!(!a.is_connected());
    if let Some(connection) = b.connection(a.address().id) {
        connection.closed().await;
    }
    store.fail_save.store(false, Ordering::SeqCst);
    pair(&a, &b).await;
    store.fail_remove.store(true, Ordering::SeqCst);
    assert!(a.devices().forget(b.endpoint_id()).is_err());
    assert_eq!(a.devices().peers().unwrap().len(), 1);
    assert!(a.is_connected());
    store.fail_remove.store(false, Ordering::SeqCst);
    a.devices().forget(b.endpoint_id()).unwrap();
    assert!(a.devices().peers().unwrap().is_empty());
    assert!(!a.is_connected());
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
}

#[derive(Debug)]
struct PausedReadStore {
    memory: MemoryStore,
    pause: AtomicBool,
    entered: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    resume: Mutex<std::sync::mpsc::Receiver<()>>,
}
impl DeviceStore for PausedReadStore {
    fn identity(&self) -> Result<SecretKey, DeviceError> {
        self.memory.identity()
    }
    fn peers(&self) -> Result<Vec<Peer>, DeviceError> {
        self.memory.peers()
    }
    fn get(&self, id: EndpointId) -> Result<Option<Peer>, DeviceError> {
        let result = self.memory.get(id);
        if self.pause.swap(false, Ordering::SeqCst) {
            self.entered
                .lock()
                .unwrap()
                .take()
                .unwrap()
                .send(())
                .unwrap();
            self.resume.lock().unwrap().recv().unwrap();
        }
        result
    }
    fn save(&self, peer: Peer) -> Result<(), DeviceError> {
        self.memory.save(peer)
    }
    fn remove(&self, id: EndpointId) -> Result<(), DeviceError> {
        self.memory.remove(id)
    }
}

#[tokio::test]
async fn forgetting_wins_against_a_reconnect_using_stale_authorization() {
    let (entered, ready) = tokio::sync::oneshot::channel();
    let (resume, paused) = std::sync::mpsc::channel();
    let store = Arc::new(PausedReadStore {
        memory: MemoryStore::new(SecretKey::from_bytes(&[11; 32])),
        pause: AtomicBool::new(false),
        entered: Mutex::new(Some(entered)),
        resume: Mutex::new(paused),
    });
    let a = MistBuilder::new(info("a"))
        .store(store.clone())
        .local_only()
        .bind()
        .await
        .unwrap();
    let b = MistBuilder::new(info("b"))
        .local_only()
        .bind()
        .await
        .unwrap();
    pair(&a, &b).await;
    let remote = b.connection(a.address().id).unwrap();
    a.connection(b.address().id)
        .unwrap()
        .close(0_u32.into(), b"test reconnect");
    remote.closed().await;
    store.pause.store(true, Ordering::SeqCst);
    let connecting = {
        let a = a.clone();
        let address = b.address();
        tokio::spawn(async move { a.connect(address).await })
    };
    tokio::time::timeout(Duration::from_secs(5), ready)
        .await
        .unwrap()
        .unwrap();
    a.devices().forget(b.endpoint_id()).unwrap();
    resume.send(()).unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(5), connecting)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    assert!(a.devices().peers().unwrap().is_empty());
    assert!(!a.is_connected());
    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
}

#[tokio::test]
async fn explicit_identity_cannot_replace_the_stored_identity() {
    let store = Arc::new(MemoryStore::new(SecretKey::from_bytes(&[12; 32])));
    let result = MistBuilder::new(info("local"))
        .store(store.clone())
        .secret_key(SecretKey::from_bytes(&[13; 32]))
        .local_only()
        .bind()
        .await;
    assert!(matches!(result, Err(RuntimeError::Storage { .. })));
    assert_eq!(
        store.identity().unwrap().public(),
        SecretKey::from_bytes(&[12; 32]).public()
    );
}

#[derive(Debug)]
struct ConstructionStore {
    memory: MemoryStore,
    entered: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    resume: Mutex<std::sync::mpsc::Receiver<()>>,
    dropped: Option<tokio::sync::oneshot::Sender<()>>,
}
impl Drop for ConstructionStore {
    fn drop(&mut self) {
        if let Some(dropped) = self.dropped.take() {
            let _ = dropped.send(());
        }
    }
}
impl DeviceStore for ConstructionStore {
    fn identity(&self) -> Result<SecretKey, DeviceError> {
        self.entered
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .send(())
            .unwrap();
        self.resume.lock().unwrap().recv().unwrap();
        self.memory.identity()
    }
    fn peers(&self) -> Result<Vec<Peer>, DeviceError> {
        self.memory.peers()
    }
    fn get(&self, id: EndpointId) -> Result<Option<Peer>, DeviceError> {
        self.memory.get(id)
    }
    fn save(&self, peer: Peer) -> Result<(), DeviceError> {
        self.memory.save(peer)
    }
    fn remove(&self, id: EndpointId) -> Result<(), DeviceError> {
        self.memory.remove(id)
    }
}

#[tokio::test]
async fn cancelled_construction_does_not_orphan_the_core_or_its_store() {
    let (entered, ready) = tokio::sync::oneshot::channel();
    let (resume, paused) = std::sync::mpsc::channel();
    let (dropped, released) = tokio::sync::oneshot::channel();
    let store = Arc::new(ConstructionStore {
        memory: MemoryStore::new(SecretKey::from_bytes(&[14; 32])),
        entered: Mutex::new(Some(entered)),
        resume: Mutex::new(paused),
        dropped: Some(dropped),
    });
    let constructing = tokio::spawn(async move {
        MistBuilder::new(info("cancelled"))
            .store(store)
            .local_only()
            .bind()
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), ready)
        .await
        .unwrap()
        .unwrap();
    constructing.abort();
    assert!(constructing.await.unwrap_err().is_cancelled());
    resume.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), released)
        .await
        .unwrap()
        .unwrap();
}
