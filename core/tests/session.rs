use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};
use std::time::Duration;

use mist_app_contract::{
    DeviceClass, DeviceInfo, DeviceStatus, MistApp, Notification, NotificationKind, Platform,
};
use mist_core::host::{MistFactory, create_mist_app};
use tempfile::TempDir;

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

async fn runtime(directory: &TempDir, name: &str) -> Arc<dyn MistApp> {
    MistFactory::new(directory.path(), device(name))
        .unwrap()
        .local_only()
        .create()
        .await
        .unwrap()
}

#[test]
fn binding_factory_does_not_require_a_host_tokio_reactor() {
    let directory = TempDir::new().unwrap();
    let runtime = block_on_without_tokio(create_mist_app(
        directory.path().to_string_lossy().into_owned(),
        device("mobile"),
    ))
    .unwrap();

    assert!(!runtime.endpoint_id().is_empty());
    block_on_without_tokio(runtime.shutdown());
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

#[tokio::test]
async fn manages_multiple_devices_and_fans_out_notifications() {
    let hub_dir = TempDir::new().unwrap();
    let phone_dir = TempDir::new().unwrap();
    let tablet_dir = TempDir::new().unwrap();
    let hub = runtime(&hub_dir, "hub").await;
    let phone = runtime(&phone_dir, "phone").await;
    let tablet = runtime(&tablet_dir, "tablet").await;

    pair(&hub, &phone).await;
    pair(&hub, &tablet).await;

    let devices = hub.device_manager().list();
    assert_eq!(devices.len(), 2);
    assert!(
        devices
            .iter()
            .all(|device| device.status == DeviceStatus::Connected && device.selected)
    );

    let phone_notifications = phone.notification_channel().subscribe();
    let tablet_notifications = tablet.notification_channel().subscribe();
    hub.notification_channel()
        .push(
            Notification {
                id: "test:1".to_owned(),
                application: "Mist tests".to_owned(),
                title: "Bridge established".to_owned(),
                body: "Notification fan-out works".to_owned(),
                timestamp_ms: 1,
                silent: true,
                kind: NotificationKind::Message,
                app_icon: None,
                image: None,
                actions: Vec::new(),
            },
            Vec::new(),
        )
        .await
        .unwrap();

    let phone_received = tokio::time::timeout(Duration::from_secs(5), phone_notifications.next())
        .await
        .unwrap()
        .unwrap();
    let tablet_received = tokio::time::timeout(Duration::from_secs(5), tablet_notifications.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(phone_received.notification.title, "Bridge established");
    assert_eq!(tablet_received.notification.title, "Bridge established");
    assert_eq!(phone_received.source_device, device("hub"));

    let tablet_id = tablet.endpoint_id();
    hub.device_manager()
        .select(tablet_id.clone(), false)
        .unwrap();
    hub.device_manager().disconnect(tablet_id).unwrap();
    let devices = hub.device_manager().list();
    assert_eq!(
        devices
            .iter()
            .find(|peer| peer.device.display_name == "tablet")
            .unwrap()
            .status,
        DeviceStatus::Disconnected
    );

    hub.shutdown().await;
    phone.shutdown().await;
    tablet.shutdown().await;
}

#[tokio::test]
async fn identity_and_device_decisions_survive_restart() {
    let hub_dir = TempDir::new().unwrap();
    let phone_dir = TempDir::new().unwrap();
    let hub = runtime(&hub_dir, "hub").await;
    let phone = runtime(&phone_dir, "phone").await;
    let endpoint_id = hub.endpoint_id();
    pair(&hub, &phone).await;
    let phone_id = phone.endpoint_id();
    hub.device_manager()
        .select(phone_id.clone(), false)
        .unwrap();
    hub.shutdown().await;
    drop(hub);

    let restarted = runtime(&hub_dir, "hub renamed").await;
    assert_eq!(restarted.endpoint_id(), endpoint_id);
    let persisted = restarted.device_manager().get(phone_id.clone()).unwrap();
    assert_eq!(persisted.device.display_name, "phone");
    assert_eq!(persisted.status, DeviceStatus::Disconnected);
    assert!(!persisted.selected);

    restarted
        .device_manager()
        .connect(phone_id.clone())
        .await
        .unwrap();
    assert_eq!(
        restarted
            .device_manager()
            .get(phone_id.clone())
            .unwrap()
            .status,
        DeviceStatus::Connected
    );

    restarted.device_manager().forget(phone_id.clone()).unwrap();
    restarted.shutdown().await;
    drop(restarted);

    let restarted = runtime(&hub_dir, "hub").await;
    assert!(restarted.device_manager().list().is_empty());
    restarted.shutdown().await;
    phone.shutdown().await;
}
