use std::sync::Arc;
use std::time::Duration;

use mist_app_contract::{
    ClipboardContent, DeviceClass, DeviceInfo, MistApp, Notification, NotificationKind, Platform,
};
use mist_core::host::MistFactory;
use tempfile::TempDir;

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

async fn paired() -> (TempDir, TempDir, Arc<dyn MistApp>, Arc<dyn MistApp>) {
    let left_dir = TempDir::new().unwrap();
    let right_dir = TempDir::new().unwrap();
    let left = runtime(&left_dir, "left").await;
    let right = runtime(&right_dir, "right").await;
    pair(&left, &right).await;
    (left_dir, right_dir, left, right)
}

#[tokio::test]
async fn text_clipboard_round_trips() {
    let (_left_dir, _right_dir, left, right) = paired().await;
    let incoming = right.clipboard().subscribe();
    let sent = ClipboardContent::Text {
        text: "hello from the left".to_owned(),
    };

    left.clipboard()
        .push(sent.clone(), Vec::new())
        .await
        .unwrap();

    let received = tokio::time::timeout(Duration::from_secs(5), incoming.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.content, sent);
    assert_eq!(received.source_device, device("left"));
    assert!(!received.sha256.is_empty());

    left.shutdown().await;
    right.shutdown().await;
}

#[tokio::test]
async fn image_blob_round_trips_to_an_app_private_file() {
    let (left_dir, _right_dir, left, right) = paired().await;
    let bytes = (0_u32..200_000)
        .map(|index| u8::try_from(index % 251).unwrap())
        .collect::<Vec<_>>();
    let path = left_dir.path().join("image.png");
    std::fs::write(&path, &bytes).unwrap();

    let incoming = right.clipboard().subscribe();
    left.clipboard()
        .push(
            ClipboardContent::Blob {
                mime_type: "image/png".to_owned(),
                path: path.to_string_lossy().into_owned(),
            },
            Vec::new(),
        )
        .await
        .unwrap();

    let received = tokio::time::timeout(Duration::from_secs(5), incoming.next())
        .await
        .unwrap()
        .unwrap();
    let ClipboardContent::Blob { mime_type, path } = received.content else {
        panic!("expected a blob");
    };
    assert_eq!(mime_type, "image/png");
    assert_eq!(std::fs::read(path).unwrap(), bytes);

    left.shutdown().await;
    right.shutdown().await;
}

#[tokio::test]
async fn echoing_received_content_is_suppressed_once() {
    let (_left_dir, _right_dir, left, right) = paired().await;
    let left_incoming = left.clipboard().subscribe();
    let right_incoming = right.clipboard().subscribe();
    let content = ClipboardContent::Text {
        text: "echo me".to_owned(),
    };

    left.clipboard()
        .push(content.clone(), Vec::new())
        .await
        .unwrap();
    let received = tokio::time::timeout(Duration::from_secs(5), right_incoming.next())
        .await
        .unwrap()
        .unwrap();

    right
        .clipboard()
        .push(received.content.clone(), Vec::new())
        .await
        .unwrap();

    assert!(
        tokio::time::timeout(Duration::from_millis(500), left_incoming.next())
            .await
            .is_err(),
        "content received from a peer must not be echoed back"
    );

    right
        .clipboard()
        .push(received.content, Vec::new())
        .await
        .unwrap();
    let deliberate = tokio::time::timeout(Duration::from_secs(5), left_incoming.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(deliberate.content, content);

    left.shutdown().await;
    right.shutdown().await;
}

#[tokio::test]
async fn explicit_targets_and_unknown_devices_are_validated() {
    let (_left_dir, _right_dir, left, right) = paired().await;
    let right_id = right.endpoint_id();
    let incoming = right.clipboard().subscribe();
    left.clipboard()
        .push(
            ClipboardContent::Text {
                text: "targeted".to_owned(),
            },
            vec![right_id],
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), incoming.next())
        .await
        .unwrap()
        .unwrap();

    let error = left
        .clipboard()
        .push(
            ClipboardContent::Text {
                text: "nobody".to_owned(),
            },
            vec!["not-an-endpoint".to_owned()],
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        mist_app_contract::ClipboardError::InvalidEndpointId { .. }
    ));

    left.shutdown().await;
    right.shutdown().await;
}

#[tokio::test]
async fn notification_and_clipboard_streams_are_independent() {
    let (_left_dir, _right_dir, left, right) = paired().await;
    let notifications = right.notification_channel().subscribe();
    let clipboard = right.clipboard().subscribe();

    left.notification_channel()
        .push(
            Notification {
                id: "test:1".to_owned(),
                application: "Mist tests".to_owned(),
                title: "notification".to_owned(),
                body: "body".to_owned(),
                timestamp_ms: 1,
                silent: true,
                kind: NotificationKind::Message,
                app_icon: None,
                image: None,
            },
            Vec::new(),
        )
        .await
        .unwrap();
    left.clipboard()
        .push(
            ClipboardContent::Text {
                text: "clipboard".to_owned(),
            },
            Vec::new(),
        )
        .await
        .unwrap();

    let notification = tokio::time::timeout(Duration::from_secs(5), notifications.next())
        .await
        .unwrap()
        .unwrap();
    let clipboard = tokio::time::timeout(Duration::from_secs(5), clipboard.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(notification.notification.title, "notification");
    assert_eq!(
        clipboard.content,
        ClipboardContent::Text {
            text: "clipboard".to_owned()
        }
    );

    left.shutdown().await;
    right.shutdown().await;
}

#[tokio::test]
async fn content_sent_while_disconnected_is_delivered_after_reconnect() {
    let (_left_dir, _right_dir, left, right) = paired().await;
    let right_id = right.endpoint_id();

    left.device_manager().disconnect(right_id.clone()).unwrap();
    let incoming = right.clipboard().subscribe();

    left.clipboard()
        .push(
            ClipboardContent::Text {
                text: "queued while offline".to_owned(),
            },
            vec![right_id],
        )
        .await
        .unwrap();

    let received = tokio::time::timeout(Duration::from_secs(15), incoming.next())
        .await
        .expect("queued clipboard content was never delivered")
        .unwrap();
    assert_eq!(
        received.content,
        ClipboardContent::Text {
            text: "queued while offline".to_owned()
        }
    );

    left.shutdown().await;
    right.shutdown().await;
}

#[tokio::test]
async fn notification_sent_while_disconnected_is_delivered_after_reconnect() {
    let (_left_dir, _right_dir, left, right) = paired().await;
    let right_id = right.endpoint_id();

    left.device_manager().disconnect(right_id.clone()).unwrap();
    let incoming = right.notification_channel().subscribe();

    left.notification_channel()
        .push(
            Notification {
                id: "test:queued".to_owned(),
                application: "Mist tests".to_owned(),
                title: "queued".to_owned(),
                body: "sent while offline".to_owned(),
                timestamp_ms: 2,
                silent: true,
                kind: NotificationKind::Message,
                app_icon: None,
                image: None,
            },
            vec![right_id],
        )
        .await
        .unwrap();

    let received = tokio::time::timeout(Duration::from_secs(15), incoming.next())
        .await
        .expect("queued notification was never delivered")
        .unwrap();
    assert_eq!(received.notification.title, "queued");

    left.shutdown().await;
    right.shutdown().await;
}

#[tokio::test]
async fn media_notifications_are_not_forwarded_by_the_notification_bridge() {
    let (_left_dir, _right_dir, left, right) = paired().await;
    let incoming = right.notification_channel().subscribe();

    for (id, kind) in [
        ("media:1", NotificationKind::Media),
        ("message:1", NotificationKind::Message),
    ] {
        left.notification_channel()
            .push(
                Notification {
                    id: id.to_owned(),
                    application: "Mist tests".to_owned(),
                    title: id.to_owned(),
                    body: "body".to_owned(),
                    timestamp_ms: 3,
                    silent: true,
                    kind,
                    app_icon: None,
                    image: None,
                },
                Vec::new(),
            )
            .await
            .unwrap();
    }

    let received = tokio::time::timeout(Duration::from_secs(5), incoming.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.notification.title, "message:1");
    assert_eq!(received.notification.kind, NotificationKind::Message);

    assert!(
        tokio::time::timeout(Duration::from_millis(500), incoming.next())
            .await
            .is_err(),
        "media notifications must not reach the generic notification stream"
    );

    left.shutdown().await;
    right.shutdown().await;
}

#[tokio::test]
async fn notification_images_cross_the_wire() {
    let (left_dir, _right_dir, left, right) = paired().await;
    let incoming = right.notification_channel().subscribe();

    let icon_bytes = (0_u32..5_000)
        .map(|index| u8::try_from(index % 251).unwrap())
        .collect::<Vec<_>>();
    let image_bytes = (0_u32..40_000)
        .map(|index| u8::try_from(index % 249).unwrap())
        .collect::<Vec<_>>();
    let icon_path = left_dir.path().join("icon.png");
    let image_path = left_dir.path().join("image.png");
    std::fs::write(&icon_path, &icon_bytes).unwrap();
    std::fs::write(&image_path, &image_bytes).unwrap();

    left.notification_channel()
        .push(
            Notification {
                id: "with-images".to_owned(),
                application: "Mist tests".to_owned(),
                title: "Rich".to_owned(),
                body: "Has artwork".to_owned(),
                timestamp_ms: 4,
                silent: true,
                kind: NotificationKind::Message,
                app_icon: Some(mist_app_contract::NotificationImage {
                    mime_type: "image/png".to_owned(),
                    path: icon_path.to_string_lossy().into_owned(),
                }),
                image: Some(mist_app_contract::NotificationImage {
                    mime_type: "image/png".to_owned(),
                    path: image_path.to_string_lossy().into_owned(),
                }),
            },
            Vec::new(),
        )
        .await
        .unwrap();

    let received = tokio::time::timeout(Duration::from_secs(5), incoming.next())
        .await
        .unwrap()
        .unwrap();
    let icon = received.notification.app_icon.expect("app icon");
    let image = received.notification.image.expect("content image");
    assert_eq!(std::fs::read(&icon.path).unwrap(), icon_bytes);
    assert_eq!(std::fs::read(&image.path).unwrap(), image_bytes);
    assert_eq!(icon.mime_type, "image/png");

    left.shutdown().await;
    right.shutdown().await;
}

#[tokio::test]
async fn oversized_notification_image_is_rejected() {
    let (left_dir, _right_dir, left, right) = paired().await;
    let incoming = right.notification_channel().subscribe();

    let path = left_dir.path().join("huge.png");
    std::fs::write(&path, vec![0_u8; 600 * 1024]).unwrap();

    let result = left
        .notification_channel()
        .push(
            Notification {
                id: "too-big".to_owned(),
                application: "Mist tests".to_owned(),
                title: "Huge".to_owned(),
                body: String::new(),
                timestamp_ms: 5,
                silent: true,
                kind: NotificationKind::Message,
                app_icon: Some(mist_app_contract::NotificationImage {
                    mime_type: "image/png".to_owned(),
                    path: path.to_string_lossy().into_owned(),
                }),
                image: None,
            },
            Vec::new(),
        )
        .await;
    assert!(result.is_err(), "oversized icon must be rejected");
    assert!(
        tokio::time::timeout(Duration::from_millis(400), incoming.next())
            .await
            .is_err()
    );

    left.shutdown().await;
    right.shutdown().await;
}

#[tokio::test]
async fn notification_image_path_is_populated_on_receive() {
    let (left_dir, _right_dir, left, right) = paired().await;
    let incoming = right.notification_channel().subscribe();
    let path = left_dir.path().join("art.png");
    std::fs::write(&path, vec![137_u8, 80, 78, 71]).unwrap();

    left.notification_channel()
        .push(
            Notification {
                id: "art".to_owned(),
                application: "Mist".to_owned(),
                title: "Art".to_owned(),
                body: String::new(),
                timestamp_ms: 6,
                silent: true,
                kind: NotificationKind::Message,
                app_icon: None,
                image: Some(mist_app_contract::NotificationImage {
                    mime_type: "image/png".to_owned(),
                    path: path.to_string_lossy().into_owned(),
                }),
            },
            Vec::new(),
        )
        .await
        .unwrap();

    let received = tokio::time::timeout(Duration::from_secs(5), incoming.next())
        .await
        .unwrap()
        .unwrap();
    let image = received.notification.image.expect("image present");
    assert!(!image.path.is_empty(), "received path must be populated");
    assert_eq!(image.mime_type, "image/png");

    left.shutdown().await;
    right.shutdown().await;
}
