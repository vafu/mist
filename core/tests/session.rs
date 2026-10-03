use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};
use std::time::Duration;

use bytes::Bytes;
use mist_core::{DeviceClass, DeviceInfo, MistBuilder, Platform, create_mist_runtime};

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
        device_class: DeviceClass::Desktop,
        platform: Platform::Linux,
    }
}

#[test]
fn binding_runtime_does_not_require_a_host_tokio_reactor() {
    let runtime = block_on_without_tokio(create_mist_runtime(device("mobile"))).unwrap();

    assert!(!runtime.endpoint_id().is_empty());
    block_on_without_tokio(runtime.shutdown());
}

#[tokio::test]
async fn binds_one_connection_for_the_session() {
    let server = MistBuilder::new(device("server"))
        .local_only()
        .bind()
        .await
        .unwrap();
    let server_addr = server.endpoint().addr();
    let server_id = server.endpoint().id();
    let accepting = {
        let server = server.clone();
        tokio::spawn(async move { server.accept().await })
    };

    let client = MistBuilder::new(device("client"))
        .local_only()
        .bind()
        .await
        .unwrap();
    let client_id = client.endpoint().id();
    let client_connection =
        tokio::time::timeout(Duration::from_secs(5), client.connect(server_addr))
            .await
            .unwrap()
            .unwrap();
    let server_connection = tokio::time::timeout(Duration::from_secs(5), accepting)
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    assert_eq!(client_connection.remote_id(), server_id);
    assert_eq!(server_connection.remote_id(), client_id);

    client_connection
        .send_datagram(Bytes::from_static(b"mist bootstrap"))
        .unwrap();
    let received = tokio::time::timeout(Duration::from_secs(5), server_connection.read_datagram())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.as_ref(), b"mist bootstrap");

    client.shutdown().await;
    server.shutdown().await;
}
