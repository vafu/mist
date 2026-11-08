use std::sync::Arc;

use iroh::{EndpointAddr, SecretKey, TransportAddr};
use mist_app_contract::{DeviceClass, DeviceInfo, Platform};

use super::{AddressSource, SqliteStateStore, StateStore, StoredAddressLookup};

fn device() -> DeviceInfo {
    DeviceInfo {
        display_name: "stored phone".to_owned(),
        device_class: DeviceClass::Phone,
        platform: Platform::Android,
    }
}

#[test]
fn stores_identity_devices_and_address_provenance() {
    let store = SqliteStateStore::in_memory().unwrap();
    let identity = [42_u8; 32];
    store.save_identity(&identity).unwrap();
    assert_eq!(store.load_identity().unwrap(), Some(identity));

    let endpoint_id = SecretKey::generate().public();
    store.upsert_device(endpoint_id, &device(), true).unwrap();
    let relay = TransportAddr::Relay("https://relay.example.test".parse().unwrap());
    let direct = TransportAddr::Ip("127.0.0.1:4242".parse().unwrap());
    store
        .add_addresses(
            endpoint_id,
            AddressSource::PairingOffer,
            std::slice::from_ref(&relay),
        )
        .unwrap();
    store
        .add_addresses(
            endpoint_id,
            AddressSource::AuthenticatedPeer,
            std::slice::from_ref(&direct),
        )
        .unwrap();
    assert_eq!(store.load_devices().unwrap().len(), 1);
    assert_eq!(store.load_addresses(endpoint_id).unwrap().len(), 2);

    store
        .remove_addresses(
            endpoint_id,
            AddressSource::PairingOffer,
            std::slice::from_ref(&relay),
        )
        .unwrap();
    assert_eq!(store.load_addresses(endpoint_id).unwrap(), vec![direct]);

    store.forget_device(endpoint_id).unwrap();
    assert!(store.load_devices().unwrap().is_empty());
    assert!(store.load_addresses(endpoint_id).unwrap().is_empty());
}

#[test]
fn stored_lookup_resolves_current_union() {
    let store = Arc::new(SqliteStateStore::in_memory().unwrap());
    let endpoint_id = SecretKey::generate().public();
    store.upsert_device(endpoint_id, &device(), true).unwrap();
    let address = TransportAddr::Ip("127.0.0.1:4343".parse().unwrap());
    store
        .add_addresses(
            endpoint_id,
            AddressSource::AuthenticatedPeer,
            std::slice::from_ref(&address),
        )
        .unwrap();
    let lookup = StoredAddressLookup::new(store);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let resolved = runtime.block_on(async {
        use iroh::address_lookup::AddressLookup;
        use n0_future::StreamExt;

        lookup
            .resolve(endpoint_id)
            .unwrap()
            .next()
            .await
            .unwrap()
            .unwrap()
            .into_endpoint_addr()
    });
    assert_eq!(resolved, EndpointAddr::from_parts(endpoint_id, [address]));
}
