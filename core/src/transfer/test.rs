use std::path::Path;

use tempfile::TempDir;

use super::{
    BlobError, BlobMetadata, BlobReader, BlobWriter, inspect_file, receive_blob, send_blob,
    validate_size,
};

#[derive(Debug, Default)]
struct MemoryWriter(Vec<u8>);

#[async_trait::async_trait]
impl BlobWriter for MemoryWriter {
    async fn write_blob_all(&mut self, buffer: &[u8]) -> Result<(), BlobError> {
        self.0.extend_from_slice(buffer);
        Ok(())
    }
}

#[derive(Debug)]
struct MemoryReader(Vec<u8>);

#[async_trait::async_trait]
impl BlobReader for MemoryReader {
    async fn read_blob_exact(&mut self, buffer: &mut [u8]) -> Result<(), BlobError> {
        if buffer.len() > self.0.len() {
            return Err(BlobError::Truncated);
        }
        buffer.copy_from_slice(&self.0[..buffer.len()]);
        self.0.drain(..buffer.len());
        Ok(())
    }
}

fn payload(size: usize) -> Vec<u8> {
    (0..size)
        .map(|index| u8::try_from(index % 251).unwrap())
        .collect()
}

fn metadata(mime_type: &str, size: u64, sha256: [u8; 32]) -> BlobMetadata {
    BlobMetadata {
        mime_type: mime_type.to_owned(),
        size,
        sha256,
    }
}

fn write_source(directory: &TempDir, bytes: &[u8]) -> std::path::PathBuf {
    let path = directory.path().join("source.bin");
    std::fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn inspects_a_file_without_loading_it_into_metadata() {
    let directory = TempDir::new().unwrap();
    let bytes = payload(super::BLOB_CHUNK_SIZE * 2 + 7);
    let path = write_source(&directory, &bytes);

    let metadata = inspect_file("application/octet-stream", &path).unwrap();

    assert_eq!(metadata.mime_type, "application/octet-stream");
    assert_eq!(metadata.size, bytes.len() as u64);
    assert_ne!(metadata.sha256, [0_u8; 32]);
}

#[tokio::test]
async fn streams_a_blob_into_a_new_file() {
    let source_directory = TempDir::new().unwrap();
    let destination_directory = TempDir::new().unwrap();
    let bytes = payload(200_000);
    let path = write_source(&source_directory, &bytes);
    let metadata = inspect_file("image/png", &path).unwrap();

    let mut writer = MemoryWriter::default();
    send_blob(&mut writer, &path, &metadata).await.unwrap();
    assert_eq!(writer.0, bytes);

    let mut reader = MemoryReader(writer.0);
    let received = receive_blob(&mut reader, destination_directory.path(), &metadata)
        .await
        .unwrap();

    assert_eq!(std::fs::read(&received).unwrap(), bytes);
    assert_eq!(
        received.extension().and_then(|value| value.to_str()),
        Some("blob")
    );
}

#[tokio::test]
async fn rejects_a_truncated_payload_and_removes_the_partial_file() {
    let directory = TempDir::new().unwrap();
    let bytes = payload(4096);
    let source_directory = TempDir::new().unwrap();
    let path = write_source(&source_directory, &bytes);
    let metadata = inspect_file("image/png", &path).unwrap();
    let mut reader = MemoryReader(bytes[..100].to_vec());

    let error = receive_blob(&mut reader, directory.path(), &metadata)
        .await
        .unwrap_err();

    assert!(matches!(error, BlobError::Truncated));
    assert!(empty(directory.path()));
}

#[tokio::test]
async fn rejects_corrupt_payload_and_removes_the_partial_file() {
    let directory = TempDir::new().unwrap();
    let bytes = payload(4096);
    let source_directory = TempDir::new().unwrap();
    let path = write_source(&source_directory, &bytes);
    let metadata = inspect_file("image/png", &path).unwrap();
    let mut corrupt = bytes;
    corrupt[0] ^= 0xff;
    let mut reader = MemoryReader(corrupt);

    let error = receive_blob(&mut reader, directory.path(), &metadata)
        .await
        .unwrap_err();

    assert!(matches!(error, BlobError::HashMismatch));
    assert!(empty(directory.path()));
}

#[test]
fn rejects_invalid_metadata() {
    assert!(matches!(validate_size(0), Err(BlobError::Empty)));
    assert!(matches!(
        validate_size(super::MAX_BLOB_SIZE + 1),
        Err(BlobError::TooLarge { .. })
    ));
    assert!(matches!(
        super::validate_mime_type("not a mime type"),
        Err(BlobError::InvalidMimeType)
    ));
}

#[test]
fn rejects_a_directory() {
    let directory = TempDir::new().unwrap();
    let error = inspect_file("image/png", Path::new(directory.path())).unwrap_err();
    assert!(matches!(error, BlobError::NotAFile));
}

fn empty(path: &Path) -> bool {
    std::fs::read_dir(path).unwrap().next().is_none()
}

#[tokio::test]
async fn streams_a_blob_over_a_real_iroh_uni_stream() {
    use iroh::endpoint::presets;
    use iroh::{Endpoint, RelayMode};

    const ALPN: &[u8] = b"mist/test/blob";

    let server = Endpoint::builder(presets::Minimal)
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .unwrap();
    let client = Endpoint::builder(presets::Minimal)
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .unwrap();

    let source_directory = TempDir::new().unwrap();
    let destination_directory = TempDir::new().unwrap();
    let bytes = payload(super::BLOB_CHUNK_SIZE * 8 + 123);
    let path = write_source(&source_directory, &bytes);
    let metadata = inspect_file("image/png", &path).unwrap();

    let server_metadata = metadata.clone();
    let destination = destination_directory.path().to_path_buf();
    let server_addr = server.addr();
    let accepting_server = server.clone();
    let receiver = tokio::spawn(async move {
        let incoming = accepting_server.accept().await.unwrap();
        let connection = incoming.await.unwrap();
        let mut stream = connection.accept_uni().await.unwrap();
        receive_blob(&mut stream, &destination, &server_metadata)
            .await
            .unwrap()
    });

    let connection = client.connect(server_addr, ALPN).await.unwrap();
    let mut stream = connection.open_uni().await.unwrap();
    send_blob(&mut stream, &path, &metadata).await.unwrap();
    stream.finish().unwrap();

    let received_path = tokio::time::timeout(std::time::Duration::from_secs(10), receiver)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(std::fs::read(received_path).unwrap(), bytes);

    client.close().await;
    server.close().await;
}

#[tokio::test]
async fn a_truncated_real_stream_is_rejected_and_leaves_no_partial_file() {
    use iroh::endpoint::presets;
    use iroh::{Endpoint, RelayMode};

    const ALPN: &[u8] = b"mist/test/blob-truncated";

    let server = Endpoint::builder(presets::Minimal)
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .unwrap();
    let client = Endpoint::builder(presets::Minimal)
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .unwrap();

    let source_directory = TempDir::new().unwrap();
    let destination_directory = TempDir::new().unwrap();
    let bytes = payload(8192);
    let path = write_source(&source_directory, &bytes);
    let metadata = inspect_file("image/png", &path).unwrap();

    let server_metadata = metadata.clone();
    let destination = destination_directory.path().to_path_buf();
    let server_addr = server.addr();
    let accepting_server = server.clone();
    let receiver = tokio::spawn(async move {
        let incoming = accepting_server.accept().await.unwrap();
        let connection = incoming.await.unwrap();
        let mut stream = connection.accept_uni().await.unwrap();
        receive_blob(&mut stream, &destination, &server_metadata).await
    });

    let connection = client.connect(server_addr, ALPN).await.unwrap();
    let mut stream = connection.open_uni().await.unwrap();
    stream.write_all(&bytes[..100]).await.unwrap();
    stream.finish().unwrap();

    let result = tokio::time::timeout(std::time::Duration::from_secs(10), receiver)
        .await
        .unwrap()
        .unwrap();

    assert!(matches!(result, Err(BlobError::Stream(_))));
    assert!(empty(destination_directory.path()));

    client.close().await;
    server.close().await;
}
