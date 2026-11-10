use std::sync::Arc;
use std::time::Duration;

use mist_app_contract::{DeviceClass, DeviceInfo, FileShare, MistApp, Platform, TransferError};
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

#[tokio::test]
async fn file_offer_and_download_round_trip() {
    let left_dir = TempDir::new().unwrap();
    let right_dir = TempDir::new().unwrap();
    let download_dir = TempDir::new().unwrap();
    let left = runtime(&left_dir, "left").await;
    let right = MistFactory::new(right_dir.path(), device("right"))
        .unwrap()
        .local_only()
        .file_inbox_directory(download_dir.path())
        .create()
        .await
        .unwrap();
    pair(&left, &right).await;

    let bytes = (0_u32..400_000)
        .map(|index| u8::try_from(index % 251).unwrap())
        .collect::<Vec<_>>();
    let source = left_dir.path().join("holiday.png");
    std::fs::write(&source, &bytes).unwrap();
    let offers = right.files().subscribe();
    let inbox = right.files().subscribe_inbox();

    let share: FileShare = left
        .files()
        .share(source.to_string_lossy().into_owned(), Vec::new())
        .await
        .unwrap();
    let offer = tokio::time::timeout(Duration::from_secs(5), offers.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(offer.id, share.id);
    assert_eq!(offer.name, "holiday.png");
    assert_eq!(offer.size, share.size);

    let path = right.files().accept(offer).await.unwrap();
    let received = tokio::time::timeout(Duration::from_secs(5), inbox.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.path, path);
    assert_eq!(received.name, "holiday.png");
    assert!(std::path::Path::new(&path).starts_with(download_dir.path()));
    assert_eq!(std::fs::read(path).unwrap(), bytes);

    left.shutdown().await;
    right.shutdown().await;
}

#[tokio::test]
async fn explicit_target_receives_the_offer_without_selection() {
    let left_dir = TempDir::new().unwrap();
    let right_dir = TempDir::new().unwrap();
    let download_dir = TempDir::new().unwrap();
    let left = runtime(&left_dir, "left").await;
    let right = MistFactory::new(right_dir.path(), device("right"))
        .unwrap()
        .local_only()
        .file_inbox_directory(download_dir.path())
        .create()
        .await
        .unwrap();
    pair(&left, &right).await;

    let bytes = b"private content".to_vec();
    let source = left_dir.path().join("private.txt");
    std::fs::write(&source, &bytes).unwrap();
    let share = left
        .files()
        .share(source.to_string_lossy().into_owned(), Vec::new())
        .await
        .unwrap();
    let forged = mist_app_contract::FileOffer {
        source_endpoint_id: left.endpoint_id(),
        source_device: device("left"),
        id: share.id,
        name: share.name,
        size: share.size,
        ticket: share.ticket,
    };

    // The receiver has the ticket, but the sender only authorized it for a
    // different endpoint after this direct second runtime bypasses the offer.
    let path = right.files().accept(forged).await;
    assert!(path.is_ok(), "the paired receiver should be authorized");

    left.shutdown().await;
    right.shutdown().await;
}

#[tokio::test]
async fn empty_and_missing_files_are_rejected() {
    let directory = TempDir::new().unwrap();
    let runtime = runtime(&directory, "left").await;
    let empty = directory.path().join("empty.bin");
    std::fs::write(&empty, b"").unwrap();

    let error = runtime
        .files()
        .share(empty.to_string_lossy().into_owned(), Vec::new())
        .await
        .unwrap_err();
    assert!(matches!(error, TransferError::Unreadable { .. }));

    let error = runtime
        .files()
        .share(
            directory
                .path()
                .join("missing.bin")
                .to_string_lossy()
                .into_owned(),
            Vec::new(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, TransferError::Unreadable { .. }));

    runtime.shutdown().await;
}

#[tokio::test]
async fn inbox_avoids_overwriting_existing_files() {
    let left_dir = TempDir::new().unwrap();
    let right_dir = TempDir::new().unwrap();
    let inbox = TempDir::new().unwrap();
    std::fs::write(inbox.path().join("same.txt"), b"existing").unwrap();
    let left = runtime(&left_dir, "left").await;
    let right = MistFactory::new(right_dir.path(), device("right"))
        .unwrap()
        .local_only()
        .file_inbox_directory(inbox.path())
        .create()
        .await
        .unwrap();
    pair(&left, &right).await;

    let source = left_dir.path().join("same.txt");
    std::fs::write(&source, b"incoming").unwrap();
    let offers = right.files().subscribe();
    left.files()
        .share(source.to_string_lossy().into_owned(), Vec::new())
        .await
        .unwrap();
    let offer = tokio::time::timeout(Duration::from_secs(5), offers.next())
        .await
        .unwrap()
        .unwrap();

    let path = right.files().accept(offer).await.unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"incoming");
    assert_eq!(
        std::fs::read(inbox.path().join("same.txt")).unwrap(),
        b"existing"
    );
    assert_eq!(
        std::path::Path::new(&path).file_name().unwrap(),
        "same (1).txt"
    );

    left.shutdown().await;
    right.shutdown().await;
}
