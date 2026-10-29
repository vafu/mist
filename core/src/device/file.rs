use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use iroh::{EndpointId, SecretKey};

use super::{DeviceError, DeviceStore, Peer, validate_peer};

/// Atomic file-backed identity and trust storage shared by Linux and Android.
///
/// Holds an exclusive OS file lock for its lifetime: one writer owns a state
/// directory. Missing files are initialized; unreadable/corrupt files are errors.
/// Identity bytes are never included in Debug output. Peer updates use atomic
/// rename, file sync and directory sync.
pub struct FileStore {
    directory: PathBuf,
    identity: SecretKey,
    peers: Mutex<()>,
    _lock: File,
}

impl std::fmt::Debug for FileStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FileStore")
            .field("directory", &self.directory)
            .finish_non_exhaustive()
    }
}

impl FileStore {
    /// Opens an installation's state directory, creating its identity if absent.
    ///
    /// # Errors
    /// Returns I/O, lock or decoding failures; never replaces invalid identity.
    pub fn open(directory: impl AsRef<Path>) -> Result<Self, DeviceError> {
        let directory = directory.as_ref().to_path_buf();
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(&directory)
            .map_err(|error| Self::io("create state directory", error))?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options
            .open(directory.join("lock"))
            .map_err(|error| Self::io("open state lock", error))?;
        lock.try_lock()
            .map_err(|error| Self::io("lock state directory", error))?;
        let identity_path = directory.join("identity");
        let bytes = match fs::read(&identity_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if directory
                    .join("peers.json")
                    .try_exists()
                    .map_err(|error| Self::io("inspect peer state", error))?
                {
                    return Err(DeviceError::InvalidData {
                        reason: "identity is missing from an existing installation".to_owned(),
                    });
                }
                let mut bytes = [0; 32];
                getrandom::fill(&mut bytes)
                    .map_err(|error| Self::io("generate local identity", error))?;
                Self::write(&directory, "identity", &bytes)?;
                bytes.to_vec()
            }
            Err(error) => return Err(Self::io("read identity", error)),
        };
        let bytes: [u8; 32] = bytes.try_into().map_err(|_| DeviceError::InvalidData {
            reason: "identity must contain exactly 32 bytes".to_owned(),
        })?;
        let store = Self {
            directory,
            identity: SecretKey::from_bytes(&bytes),
            peers: Mutex::new(()),
            _lock: lock,
        };
        store.load()?;
        Ok(store)
    }

    fn io(operation: &str, error: impl std::fmt::Display) -> DeviceError {
        DeviceError::Io {
            reason: format!("{operation}: {error}"),
        }
    }

    fn write(directory: &Path, name: &str, bytes: &[u8]) -> Result<(), DeviceError> {
        let mut temporary = tempfile::NamedTempFile::new_in(directory)
            .map_err(|error| Self::io("create temporary state", error))?;
        temporary
            .write_all(bytes)
            .map_err(|error| Self::io("write state", error))?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|error| Self::io("sync state", error))?;
        temporary
            .persist(directory.join(name))
            .map_err(|error| Self::io("replace state", error))?;
        File::open(directory)
            .and_then(|file| file.sync_all())
            .map_err(|error| Self::io("sync state directory", error))
    }

    fn load(&self) -> Result<BTreeMap<String, Peer>, DeviceError> {
        let bytes = match fs::read(self.directory.join("peers.json")) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(BTreeMap::new());
            }
            Err(error) => return Err(Self::io("read peers", error)),
        };
        let peers: BTreeMap<String, Peer> =
            serde_json::from_slice(&bytes).map_err(|error| DeviceError::InvalidData {
                reason: format!("decode peers: {error}"),
            })?;
        for (id, peer) in &peers {
            validate_peer(peer)?;
            if id != &peer.endpoint_id {
                return Err(DeviceError::InvalidData {
                    reason: "peer key does not match identity".to_owned(),
                });
            }
        }
        Ok(peers)
    }
}

impl DeviceStore for FileStore {
    fn identity(&self) -> Result<SecretKey, DeviceError> {
        Ok(self.identity.clone())
    }
    fn peers(&self) -> Result<Vec<Peer>, DeviceError> {
        let _guard = self.peers.lock().expect("peer store poisoned");
        Ok(self.load()?.into_values().collect())
    }
    fn get(&self, id: EndpointId) -> Result<Option<Peer>, DeviceError> {
        let _guard = self.peers.lock().expect("peer store poisoned");
        Ok(self.load()?.remove(&id.to_string()))
    }
    fn save(&self, peer: Peer) -> Result<(), DeviceError> {
        validate_peer(&peer)?;
        let _guard = self.peers.lock().expect("peer store poisoned");
        let mut peers = self.load()?;
        peers.insert(peer.endpoint_id.clone(), peer);
        let bytes = serde_json::to_vec(&peers).map_err(|error| DeviceError::InvalidData {
            reason: error.to_string(),
        })?;
        Self::write(&self.directory, "peers.json", &bytes)
    }
    fn remove(&self, id: EndpointId) -> Result<(), DeviceError> {
        let _guard = self.peers.lock().expect("peer store poisoned");
        let mut peers = self.load()?;
        peers.remove(&id.to_string());
        let bytes = serde_json::to_vec(&peers).map_err(|error| DeviceError::InvalidData {
            reason: error.to_string(),
        })?;
        Self::write(&self.directory, "peers.json", &bytes)
    }
}
