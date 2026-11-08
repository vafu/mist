//! Streaming binary payload transfer shared by future features.
//!
//! Blob metadata travels in a control frame; payload bytes follow on the same
//! stream. Files are streamed and hashed in bounded chunks so neither side has
//! to hold a payload in memory.

#![allow(
    dead_code,
    reason = "the blob primitive is landed before its first feature consumer"
)]

#[cfg(test)]
mod test;

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use iroh::endpoint::{RecvStream, SendStream};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Largest binary payload accepted by the shared transfer layer.
pub(crate) const MAX_BLOB_SIZE: u64 = 32 * 1024 * 1024;
/// Streaming chunk size used for hashing and file I/O.
pub(crate) const BLOB_CHUNK_SIZE: usize = 64 * 1024;
/// Largest accepted MIME type length.
const MAX_MIME_TYPE_LENGTH: usize = 255;

/// Validated metadata describing one binary payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BlobMetadata {
    pub(crate) mime_type: String,
    pub(crate) size: u64,
    pub(crate) sha256: [u8; 32],
}

/// Failure while reading, validating, or writing a binary payload.
#[derive(Debug, Error)]
pub(crate) enum BlobError {
    #[error("invalid blob MIME type")]
    InvalidMimeType,
    #[error("blob is empty")]
    Empty,
    #[error("blob size {size} exceeds the {MAX_BLOB_SIZE}-byte limit")]
    TooLarge { size: u64 },
    #[error("blob path is not a regular file")]
    NotAFile,
    #[error("blob size changed after validation")]
    SizeChanged,
    #[error("blob content hash does not match its metadata")]
    HashMismatch,
    #[error("blob transfer ended before the declared size")]
    Truncated,
    #[error("blob transfer failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("blob stream failed: {0}")]
    Stream(String),
}

/// Minimal readable byte stream used by [`receive_blob`].
#[async_trait]
pub(crate) trait BlobReader: Send {
    async fn read_blob_exact(&mut self, buffer: &mut [u8]) -> Result<(), BlobError>;
}

/// Minimal writable byte stream used by [`send_blob`].
#[async_trait]
pub(crate) trait BlobWriter: Send {
    async fn write_blob_all(&mut self, buffer: &[u8]) -> Result<(), BlobError>;
}

#[async_trait]
impl BlobReader for RecvStream {
    async fn read_blob_exact(&mut self, buffer: &mut [u8]) -> Result<(), BlobError> {
        self.read_exact(buffer)
            .await
            .map_err(|error| BlobError::Stream(error.to_string()))
    }
}

#[async_trait]
impl BlobWriter for SendStream {
    async fn write_blob_all(&mut self, buffer: &[u8]) -> Result<(), BlobError> {
        self.write_all(buffer)
            .await
            .map_err(|error| BlobError::Stream(error.to_string()))
    }
}

/// Validates a peer-provided MIME type.
///
/// Feature layers apply their own allow-list on top of this check.
pub(crate) fn validate_mime_type(mime_type: &str) -> Result<(), BlobError> {
    if mime_type.is_empty()
        || mime_type.len() > MAX_MIME_TYPE_LENGTH
        || mime_type.trim() != mime_type
        || !mime_type.contains('/')
        || mime_type.chars().any(char::is_control)
    {
        return Err(BlobError::InvalidMimeType);
    }
    Ok(())
}

/// Validates a peer-provided payload size.
pub(crate) fn validate_size(size: u64) -> Result<(), BlobError> {
    if size == 0 {
        return Err(BlobError::Empty);
    }
    if size > MAX_BLOB_SIZE {
        return Err(BlobError::TooLarge { size });
    }
    Ok(())
}

/// Builds metadata for a local file, hashing it without loading it into memory.
pub(crate) fn inspect_file(mime_type: &str, path: &Path) -> Result<BlobMetadata, BlobError> {
    validate_mime_type(mime_type)?;
    let metadata = std::fs::metadata(path)?;
    if !metadata.is_file() {
        return Err(BlobError::NotAFile);
    }
    validate_size(metadata.len())?;

    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; BLOB_CHUNK_SIZE];
    loop {
        let read = std::io::Read::read(&mut file, &mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(BlobMetadata {
        mime_type: mime_type.to_owned(),
        size: metadata.len(),
        sha256: hasher.finalize().into(),
    })
}

/// Streams one local file as the payload described by `metadata`.
///
/// The caller writes the metadata control frame first and finishes the stream
/// after this function returns successfully.
pub(crate) async fn send_blob(
    writer: &mut impl BlobWriter,
    path: &Path,
    metadata: &BlobMetadata,
) -> Result<(), BlobError> {
    validate_mime_type(&metadata.mime_type)?;
    validate_size(metadata.size)?;
    let size = std::fs::metadata(path)?.len();
    if !std::fs::metadata(path)?.is_file() {
        return Err(BlobError::NotAFile);
    }
    if size != metadata.size {
        return Err(BlobError::SizeChanged);
    }

    let mut file = File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut remaining = metadata.size;
    let mut buffer = vec![0_u8; BLOB_CHUNK_SIZE];
    while remaining > 0 {
        let limit = usize::try_from(remaining.min(BLOB_CHUNK_SIZE as u64))
            .map_err(|_| BlobError::TooLarge { size: remaining })?;
        let read = file.read(&mut buffer[..limit]).await?;
        if read == 0 {
            return Err(BlobError::Truncated);
        }
        hasher.update(&buffer[..read]);
        writer.write_blob_all(&buffer[..read]).await?;
        remaining -= read as u64;
    }

    if <[u8; 32]>::from(hasher.finalize()) != metadata.sha256 {
        return Err(BlobError::HashMismatch);
    }
    Ok(())
}

/// Receives one payload into an app-private file and returns its path.
///
/// The destination file is removed if validation, streaming, or hashing fails.
pub(crate) async fn receive_blob(
    reader: &mut impl BlobReader,
    directory: &Path,
    metadata: &BlobMetadata,
) -> Result<PathBuf, BlobError> {
    validate_mime_type(&metadata.mime_type)?;
    validate_size(metadata.size)?;
    tokio::fs::create_dir_all(directory).await?;

    let path = directory.join(format!("{}.blob", random_hex()?));
    let result = write_payload(reader, &path, metadata).await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&path).await;
    }
    result?;
    Ok(path)
}

async fn write_payload(
    reader: &mut impl BlobReader,
    path: &Path,
    metadata: &BlobMetadata,
) -> Result<(), BlobError> {
    let mut file = File::create(path).await?;
    let mut hasher = Sha256::new();
    let mut remaining = metadata.size;
    let mut buffer = vec![0_u8; BLOB_CHUNK_SIZE];
    while remaining > 0 {
        let limit = usize::try_from(remaining.min(BLOB_CHUNK_SIZE as u64))
            .map_err(|_| BlobError::TooLarge { size: remaining })?;
        reader.read_blob_exact(&mut buffer[..limit]).await?;
        hasher.update(&buffer[..limit]);
        file.write_all(&buffer[..limit]).await?;
        remaining -= limit as u64;
    }
    file.flush().await?;

    if <[u8; 32]>::from(hasher.finalize()) != metadata.sha256 {
        return Err(BlobError::HashMismatch);
    }
    Ok(())
}

fn random_hex() -> Result<String, BlobError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| BlobError::Stream(error.to_string()))?;
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        write!(&mut hex, "{byte:02x}").map_err(|error| BlobError::Stream(error.to_string()))?;
    }
    Ok(hex)
}
