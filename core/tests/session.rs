use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};
use std::time::Duration;

use bytes::Bytes;
use mist_core::{
    DeviceInfo, MistBuilder, PairingError, Platform, Runtime, RuntimeError, create_mist_runtime,
};

struct ThreadWake(Thread);
impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

fn block_on_without_tokio<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => thread::park(),
        }
    }
}

fn device(name: &str) -> DeviceInfo {
    DeviceInfo {
        display_name: name.to_owned(),
        platform: Platform::Linux,
    }
}

async fn pair(first: &dyn Runtime, second: &dyn Runtime) {
    let offering = first.pairing();
    let joining = second.pairing();
    let offer = offering.initiate().unwrap();
    let (a, b) = tokio::join!(offering.wait_for_peer(), joining.accept(offer));
    a.unwrap();
    b.unwrap();
}

async fn disconnect(first: &dyn Runtime, second: &dyn Runtime) {
    let a = first.connection(second.address().id).unwrap();
    let b = second.connection(first.address().id).unwrap();
    a.close(0_u32.into(), b"test reconnect");
    b.closed().await;
}

#[test]
fn binding_runtime_does_not_require_a_host_tokio_reactor() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = block_on_without_tokio(create_mist_runtime(
        device("adapter"),
        directory.path().to_str().unwrap().to_owned(),
    ))
    .unwrap();
    assert!(!runtime.endpoint_id().is_empty());
    block_on_without_tokio(runtime.shutdown()).unwrap();
}

#[test]
fn core_operations_outlive_the_callers_tokio_runtime() {
    let host = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let first = host
        .block_on(MistBuilder::new(device("first")).local_only().bind())
        .unwrap();
    drop(host);
    let second =
        block_on_without_tokio(MistBuilder::new(device("second")).local_only().bind()).unwrap();
    block_on_without_tokio(pair(first.as_ref(), second.as_ref()));
    let outgoing = block_on_without_tokio(first.connect(second.address())).unwrap();
    let incoming = second.connection(first.address().id).unwrap();
    outgoing
        .send_datagram(Bytes::from_static(b"independent execution"))
        .unwrap();
    assert_eq!(
        block_on_without_tokio(incoming.read_datagram())
            .unwrap()
            .as_ref(),
        b"independent execution"
    );
    block_on_without_tokio(first.shutdown()).unwrap();
    let third =
        block_on_without_tokio(MistBuilder::new(device("third")).local_only().bind()).unwrap();
    let offer = second.pairing().initiate().unwrap();
    let waiting = second.pairing();
    let joining = third.pairing();
    let (waited, joined) = block_on_without_tokio(async {
        tokio::join!(waiting.wait_for_peer(), joining.accept(offer))
    });
    waited.unwrap();
    joined.unwrap();
    block_on_without_tokio(second.shutdown()).unwrap();
    block_on_without_tokio(third.shutdown()).unwrap();
}

#[tokio::test]
async fn runtime_contract_carries_peer_traffic() {
    let first: Arc<dyn Runtime> = MistBuilder::new(device("first"))
        .local_only()
        .bind()
        .await
        .unwrap();
    let second: Arc<dyn Runtime> = MistBuilder::new(device("second"))
        .local_only()
        .bind()
        .await
        .unwrap();
    pair(first.as_ref(), second.as_ref()).await;
    let connection = first.connect(second.address()).await.unwrap();
    let incoming = second.connection(first.address().id).unwrap();
    connection
        .send_datagram(Bytes::from_static(b"mist"))
        .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), incoming.read_datagram())
            .await
            .unwrap()
            .unwrap()
            .as_ref(),
        b"mist"
    );
    first.shutdown().await.unwrap();
    second.shutdown().await.unwrap();
}

#[tokio::test]
async fn peer_connections_are_independent_and_closed_connections_can_be_replaced() {
    let identity = iroh::SecretKey::from_bytes(&[9; 32]);
    let store = Arc::new(mist_core::MemoryStore::new(identity.clone()));
    let first = MistBuilder::new(device("first"))
        .secret_key(identity.clone())
        .store(store.clone())
        .local_only()
        .bind()
        .await
        .unwrap();
    let middle = MistBuilder::new(device("middle"))
        .local_only()
        .bind()
        .await
        .unwrap();
    let last = MistBuilder::new(device("last"))
        .local_only()
        .bind()
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        pair(first.as_ref(), middle.as_ref()).await;
        pair(middle.as_ref(), last.as_ref()).await;
        let old = middle.connection(first.address().id).unwrap();
        first.shutdown().await.unwrap();
        old.closed().await;
        assert!(middle.connection(first.address().id).is_none());
        assert!(middle.connection(last.address().id).is_some());
        let replacement = MistBuilder::new(device("replacement"))
            .secret_key(identity)
            .store(store)
            .local_only()
            .bind()
            .await
            .unwrap();
        replacement.connect(middle.address()).await.unwrap();
        assert!(middle.connection(replacement.address().id).is_some());
        assert!(middle.is_connected());
        replacement.shutdown().await.unwrap();
        last.shutdown().await.unwrap();
        middle.shutdown().await.unwrap();
        assert!(!middle.is_connected());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn simultaneous_connections_converge_on_the_same_transport() {
    let first = MistBuilder::new(device("first"))
        .local_only()
        .bind()
        .await
        .unwrap();
    let second = MistBuilder::new(device("second"))
        .local_only()
        .bind()
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        pair(first.as_ref(), second.as_ref()).await;
        disconnect(first.as_ref(), second.as_ref()).await;
        let (a, b, c) = tokio::join!(
            first.connect(second.address()),
            second.connect(first.address()),
            first.connect(second.address())
        );
        assert!(a.is_ok() || b.is_ok() || c.is_ok());
        let a = first.connect(second.address()).await.unwrap();
        let b = second.connect(first.address()).await.unwrap();
        let mut a_key = [0; 32];
        let mut b_key = [0; 32];
        a.export_keying_material(&mut a_key, b"test/identity", b"")
            .unwrap();
        b.export_keying_material(&mut b_key, b"test/identity", b"")
            .unwrap();
        assert_eq!(a_key, b_key);
        a.send_datagram(Bytes::from_static(b"winner")).unwrap();
        assert_eq!(b.read_datagram().await.unwrap().as_ref(), b"winner");
        first.shutdown().await.unwrap();
        second.shutdown().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn shutdown_finishes_pending_pairing_and_rejects_new_work() {
    let first = MistBuilder::new(device("first"))
        .local_only()
        .bind()
        .await
        .unwrap();
    let second = MistBuilder::new(device("second"))
        .local_only()
        .bind()
        .await
        .unwrap();
    let pairing = first.pairing();
    pairing.initiate().unwrap();
    let waiting = pairing.wait_for_peer();
    tokio::pin!(waiting);
    std::future::poll_fn(|cx| {
        assert!(waiting.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    let (a, b) = tokio::join!(first.shutdown(), first.shutdown());
    a.unwrap();
    b.unwrap();
    assert!(matches!(waiting.await, Err(PairingError::RuntimeClosed)));
    assert!(matches!(
        pairing.initiate(),
        Err(PairingError::RuntimeClosed)
    ));
    assert!(matches!(
        first.connect(second.address()).await,
        Err(RuntimeError::Closed)
    ));
    let third = MistBuilder::new(device("third"))
        .local_only()
        .bind()
        .await
        .unwrap();
    pair(second.as_ref(), third.as_ref()).await;
    second.connect(third.address()).await.unwrap();
    second.shutdown().await.unwrap();
    third.shutdown().await.unwrap();
}
