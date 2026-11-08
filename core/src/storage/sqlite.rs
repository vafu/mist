use std::net::SocketAddr;
use std::path::Path;
use std::str::FromStr;
use std::sync::{Mutex, PoisonError};

use iroh::{EndpointId, RelayUrl, TransportAddr};
use iroh_base::CustomAddr;
use mist_app_contract::{DeviceClass, DeviceInfo, Platform};
use rusqlite::{Connection, OptionalExtension, params};

use super::{AddressSource, StateStore, StoreError, StoredDevice};

#[derive(Debug)]
pub(crate) struct SqliteStateStore {
    connection: Mutex<Connection>,
}

impl SqliteStateStore {
    pub(crate) fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(StoreError::CreateDirectory)?;
        }
        Self::from_connection(Connection::open(path)?)
    }

    pub(crate) fn in_memory() -> Result<Self, StoreError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Self, StoreError> {
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS local_identity (
                 singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                 secret_key BLOB NOT NULL CHECK (length(secret_key) = 32)
             );
             CREATE TABLE IF NOT EXISTS devices (
                 endpoint_id TEXT PRIMARY KEY NOT NULL,
                 display_name TEXT NOT NULL,
                 device_class INTEGER NOT NULL,
                 platform INTEGER NOT NULL,
                 selected INTEGER NOT NULL CHECK (selected IN (0, 1))
             );
             CREATE TABLE IF NOT EXISTS addresses (
                 endpoint_id TEXT NOT NULL REFERENCES devices(endpoint_id) ON DELETE CASCADE,
                 source TEXT NOT NULL,
                 kind INTEGER NOT NULL,
                 value TEXT NOT NULL,
                 PRIMARY KEY(endpoint_id, source, kind, value)
             );",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn connection(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.connection
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl StateStore for SqliteStateStore {
    fn load_identity(&self) -> Result<Option<[u8; 32]>, StoreError> {
        let bytes = self
            .connection()
            .query_row(
                "SELECT secret_key FROM local_identity WHERE singleton = 1",
                [],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;
        bytes
            .map(|bytes| {
                bytes.try_into().map_err(|bytes: Vec<u8>| {
                    StoreError::CorruptIdentity(format!(
                        "expected 32 secret-key bytes, found {}",
                        bytes.len()
                    ))
                })
            })
            .transpose()
    }

    fn save_identity(&self, identity: &[u8; 32]) -> Result<(), StoreError> {
        self.connection().execute(
            "INSERT INTO local_identity(singleton, secret_key) VALUES (1, ?1)
             ON CONFLICT(singleton) DO NOTHING",
            [identity.as_slice()],
        )?;
        let stored = self
            .load_identity()?
            .ok_or_else(|| StoreError::CorruptIdentity("identity disappeared after save".into()))?;
        if stored != *identity {
            return Err(StoreError::CorruptIdentity(
                "another identity already exists".into(),
            ));
        }
        Ok(())
    }

    fn load_devices(&self) -> Result<Vec<StoredDevice>, StoreError> {
        let connection = self.connection();
        let mut statement = connection.prepare(
            "SELECT endpoint_id, display_name, device_class, platform, selected FROM devices",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, bool>(4)?,
            ))
        })?;
        rows.map(|row| {
            let (endpoint, display_name, class, platform, selected) = row?;
            let endpoint_id =
                EndpointId::from_str(&endpoint).map_err(|error| StoreError::CorruptDevice {
                    endpoint_id: endpoint.clone(),
                    reason: error.to_string(),
                })?;
            Ok(StoredDevice {
                endpoint_id,
                device: DeviceInfo {
                    display_name,
                    device_class: decode_class(class).ok_or_else(|| StoreError::CorruptDevice {
                        endpoint_id: endpoint.clone(),
                        reason: format!("unknown device class {class}"),
                    })?,
                    platform: decode_platform(platform).ok_or_else(|| {
                        StoreError::CorruptDevice {
                            endpoint_id: endpoint,
                            reason: format!("unknown platform {platform}"),
                        }
                    })?,
                },
                selected,
            })
        })
        .collect()
    }

    fn upsert_device(
        &self,
        endpoint_id: EndpointId,
        device: &DeviceInfo,
        selected_if_new: bool,
    ) -> Result<(), StoreError> {
        self.connection().execute(
            "INSERT INTO devices(endpoint_id, display_name, device_class, platform, selected)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(endpoint_id) DO UPDATE SET
                 display_name = excluded.display_name,
                 device_class = excluded.device_class,
                 platform = excluded.platform",
            params![
                endpoint_id.to_string(),
                device.display_name,
                encode_class(device.device_class),
                encode_platform(device.platform),
                selected_if_new,
            ],
        )?;
        Ok(())
    }

    fn set_selected(&self, endpoint_id: EndpointId, selected: bool) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE devices SET selected = ?2 WHERE endpoint_id = ?1",
            params![endpoint_id.to_string(), selected],
        )?;
        Ok(())
    }

    fn forget_device(&self, endpoint_id: EndpointId) -> Result<(), StoreError> {
        self.connection().execute(
            "DELETE FROM devices WHERE endpoint_id = ?1",
            [endpoint_id.to_string()],
        )?;
        Ok(())
    }

    fn load_addresses(&self, endpoint_id: EndpointId) -> Result<Vec<TransportAddr>, StoreError> {
        let endpoint = endpoint_id.to_string();
        let connection = self.connection();
        let mut statement = connection.prepare(
            "SELECT kind, value FROM addresses WHERE endpoint_id = ?1 ORDER BY source, kind, value",
        )?;
        let rows = statement.query_map([&endpoint], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.map(|row| {
            let (kind, value) = row?;
            decode_address(kind, &value).map_err(|reason| StoreError::CorruptAddress {
                endpoint_id: endpoint.clone(),
                reason,
            })
        })
        .collect()
    }

    fn load_addresses_from(
        &self,
        endpoint_id: EndpointId,
        source: AddressSource,
    ) -> Result<Vec<TransportAddr>, StoreError> {
        let endpoint = endpoint_id.to_string();
        let connection = self.connection();
        let mut statement = connection.prepare(
            "SELECT kind, value FROM addresses
             WHERE endpoint_id = ?1 AND source = ?2 ORDER BY kind, value",
        )?;
        let rows = statement.query_map(params![endpoint, source.as_str()], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.map(|row| {
            let (kind, value) = row?;
            decode_address(kind, &value).map_err(|reason| StoreError::CorruptAddress {
                endpoint_id: endpoint.clone(),
                reason,
            })
        })
        .collect()
    }

    fn add_addresses(
        &self,
        endpoint_id: EndpointId,
        source: AddressSource,
        addresses: &[TransportAddr],
    ) -> Result<(), StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        for address in addresses {
            let (kind, value) = encode_address(address);
            transaction.execute(
                "INSERT OR IGNORE INTO addresses(endpoint_id, source, kind, value)
                 VALUES (?1, ?2, ?3, ?4)",
                params![endpoint_id.to_string(), source.as_str(), kind, value],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    fn remove_addresses(
        &self,
        endpoint_id: EndpointId,
        source: AddressSource,
        addresses: &[TransportAddr],
    ) -> Result<(), StoreError> {
        let mut connection = self.connection();
        let transaction = connection.transaction()?;
        for address in addresses {
            let (kind, value) = encode_address(address);
            transaction.execute(
                "DELETE FROM addresses
                 WHERE endpoint_id = ?1 AND source = ?2 AND kind = ?3 AND value = ?4",
                params![endpoint_id.to_string(), source.as_str(), kind, value],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }
}

fn encode_class(class: DeviceClass) -> i64 {
    match class {
        DeviceClass::Phone => 1,
        DeviceClass::Tablet => 2,
        DeviceClass::Laptop => 3,
        DeviceClass::Desktop => 4,
        DeviceClass::Server => 5,
        DeviceClass::Other => 6,
    }
}

fn decode_class(value: i64) -> Option<DeviceClass> {
    Some(match value {
        1 => DeviceClass::Phone,
        2 => DeviceClass::Tablet,
        3 => DeviceClass::Laptop,
        4 => DeviceClass::Desktop,
        5 => DeviceClass::Server,
        6 => DeviceClass::Other,
        _ => return None,
    })
}

fn encode_platform(platform: Platform) -> i64 {
    match platform {
        Platform::Android => 1,
        Platform::Linux => 2,
        Platform::Ios => 3,
        Platform::Macos => 4,
        Platform::Windows => 5,
        Platform::Other => 6,
    }
}

fn decode_platform(value: i64) -> Option<Platform> {
    Some(match value {
        1 => Platform::Android,
        2 => Platform::Linux,
        3 => Platform::Ios,
        4 => Platform::Macos,
        5 => Platform::Windows,
        6 => Platform::Other,
        _ => return None,
    })
}

fn encode_address(address: &TransportAddr) -> (i64, String) {
    match address {
        TransportAddr::Relay(value) => (1, value.to_string()),
        TransportAddr::Ip(value) => (2, value.to_string()),
        TransportAddr::Custom(value) => (3, value.to_string()),
        _ => (0, address.to_string()),
    }
}

fn decode_address(kind: i64, value: &str) -> Result<TransportAddr, String> {
    match kind {
        1 => RelayUrl::from_str(value)
            .map(TransportAddr::Relay)
            .map_err(|error| error.to_string()),
        2 => SocketAddr::from_str(value)
            .map(TransportAddr::Ip)
            .map_err(|error| error.to_string()),
        3 => CustomAddr::from_str(value)
            .map(TransportAddr::Custom)
            .map_err(|error| error.to_string()),
        _ => Err(format!("unknown address kind {kind}")),
    }
}
