use std::sync::Arc;

use iroh::SecretKey;

use super::{DeviceError, DeviceInfo, DeviceStore, FileStore, MemoryStore, Peer, Platform};

fn peer(seed: u8, name: &str) -> Peer {
    Peer {
        endpoint_id: SecretKey::from_bytes(&[seed; 32]).public().to_string(),
        device: DeviceInfo {
            display_name: name.to_owned(),
            platform: Platform::Android,
        },
    }
}

#[test]
fn persistent_identity_and_peers_survive_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let identity;
    let first = peer(2, "phone");
    {
        let store = FileStore::open(directory.path()).unwrap();
        identity = store.identity().unwrap().public();
        store.save(first.clone()).unwrap();
        store.save(peer(2, "renamed phone")).unwrap();
        assert_eq!(store.peers().unwrap().len(), 1);
    }
    let store = FileStore::open(directory.path()).unwrap();
    assert_eq!(identity, store.identity().unwrap().public());
    assert_eq!(store.peers().unwrap(), vec![peer(2, "renamed phone")]);
    store.remove(first.endpoint_id.parse().unwrap()).unwrap();
    assert!(store.peers().unwrap().is_empty());
}

#[test]
fn corrupt_identity_or_trust_is_not_overwritten() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("identity"), b"invalid").unwrap();
    assert!(matches!(
        FileStore::open(directory.path()),
        Err(DeviceError::InvalidData { .. })
    ));
    assert_eq!(
        std::fs::read(directory.path().join("identity")).unwrap(),
        b"invalid"
    );
    std::fs::remove_file(directory.path().join("identity")).unwrap();
    {
        let _store = FileStore::open(directory.path()).unwrap();
    }
    std::fs::write(directory.path().join("peers.json"), b"invalid").unwrap();
    assert!(matches!(
        FileStore::open(directory.path()),
        Err(DeviceError::InvalidData { .. })
    ));
    assert_eq!(
        std::fs::read(directory.path().join("peers.json")).unwrap(),
        b"invalid"
    );
    std::fs::remove_file(directory.path().join("identity")).unwrap();
    assert!(matches!(
        FileStore::open(directory.path()),
        Err(DeviceError::InvalidData { .. })
    ));
    assert!(!directory.path().join("identity").exists());
}

#[test]
fn one_writer_owns_each_installation_and_concurrent_updates_are_preserved() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(FileStore::open(directory.path()).unwrap());
    assert!(FileStore::open(directory.path()).is_err());
    let a = Arc::clone(&store);
    let b = Arc::clone(&store);
    let first = std::thread::spawn(move || a.save(peer(3, "first")).unwrap());
    let second = std::thread::spawn(move || b.save(peer(4, "second")).unwrap());
    first.join().unwrap();
    second.join().unwrap();
    assert_eq!(store.peers().unwrap().len(), 2);
    drop(store);
    assert_eq!(
        FileStore::open(directory.path())
            .unwrap()
            .peers()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn storage_implementations_share_peer_contract() {
    let directory = tempfile::tempdir().unwrap();
    let stores: Vec<Box<dyn DeviceStore>> = vec![
        Box::new(MemoryStore::new(SecretKey::from_bytes(&[1; 32]))),
        Box::new(FileStore::open(directory.path()).unwrap()),
    ];
    for store in stores {
        let record = peer(5, "remote");
        let id = record.endpoint_id.parse().unwrap();
        store.save(record.clone()).unwrap();
        assert_eq!(store.get(id).unwrap(), Some(record));
        store.remove(id).unwrap();
        store.remove(id).unwrap();
        assert!(store.get(id).unwrap().is_none());
        let mut invalid = peer(5, "remote");
        invalid.endpoint_id = "bad".to_owned();
        assert!(store.save(invalid).is_err());
    }
}
