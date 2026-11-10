use std::path::PathBuf;

use async_trait::async_trait;
use iroh::SecretKey;
use iroh_blobs::Hash;

use super::{BlobStore, ShareAuthorization, StoredBlob, TransferError};

#[derive(Debug)]
struct TestBlobStore;

#[async_trait]
impl BlobStore for TestBlobStore {
    async fn import_file(&self, path: PathBuf) -> Result<StoredBlob, TransferError> {
        Err(TransferError::NotAFile {
            path: path.display().to_string(),
        })
    }

    async fn download(&self, provider: iroh::EndpointId, hash: Hash) -> Result<(), TransferError> {
        Err(TransferError::Transport {
            reason: format!("{provider} {hash}"),
        })
    }

    async fn export(
        &self,
        hash: Hash,
        destination: PathBuf,
        file_name: &str,
    ) -> Result<PathBuf, TransferError> {
        Ok(destination.join(format!("{file_name}-{hash}")))
    }

    async fn shutdown(&self) -> Result<(), TransferError> {
        Ok(())
    }

    async fn serve(&self, _connection: iroh::endpoint::Connection) {}

    fn authorize(&self, _endpoint_id: iroh::EndpointId, _hash: Hash) {}

    fn ticket(&self, hash: Hash) -> iroh_blobs::ticket::BlobTicket {
        iroh_blobs::ticket::BlobTicket::new(
            iroh::EndpointAddr::new(SecretKey::generate().public()),
            hash,
            iroh_blobs::BlobFormat::Raw,
        )
    }
}

#[test]
fn authorization_is_scoped_to_endpoint_and_hash() {
    let authorization = ShareAuthorization::default();
    let first = SecretKey::generate().public();
    let second = SecretKey::generate().public();
    let hash = Hash::new(b"trusted content");
    let other_hash = Hash::new(b"other content");

    authorization.allow(first, hash);

    assert!(authorization.allows(first, hash));
    assert!(!authorization.allows(second, hash));
    assert!(!authorization.allows(first, other_hash));

    assert!(!authorization.allows(first, other_hash));
}

#[tokio::test]
async fn test_store_can_be_shut_down() {
    TestBlobStore.shutdown().await.unwrap();
}
