//! Development probe for the shared blob streaming primitive.
//!
//! This is intentionally feature-gated and not part of the application API. It
//! lets a Linux host and an Android device exchange a real blob over Iroh so the
//! transfer primitive can be validated across platforms before a feature wires
//! it in.

use std::io::Write;
use std::path::PathBuf;
use std::str::FromStr;

use iroh::endpoint::{Connection, RecvStream, SendStream, presets};
use iroh::{Endpoint, EndpointAddr, RelayMode, TransportAddr};
use iroh_tickets::endpoint::EndpointTicket;
use thiserror::Error;

use crate::transfer::{BlobMetadata, inspect_file, receive_blob, send_blob, validate_mime_type};

const PROBE_ALPN: &[u8] = b"mist/blob-probe/1";
const HEADER_MAGIC: &[u8; 8] = b"MISTBLB1";

/// Failure while running a blob probe.
#[derive(Debug, Error)]
pub enum ProbeError {
    #[error("usage: listen <output-directory> | send <ticket> <file> [mime-type]")]
    Usage,
    #[error("invalid probe argument: {0}")]
    Argument(String),
    #[error("failed to bind the probe endpoint: {0}")]
    Bind(String),
    #[error("failed to parse the probe ticket: {0}")]
    Ticket(String),
    #[error("probe connection failed: {0}")]
    Connection(String),
    #[error("probe protocol failed: {0}")]
    Protocol(String),
    #[error("blob transfer failed: {0}")]
    Transfer(String),
    #[error("probe I/O failed: {0}")]
    Io(String),
}

/// Listens for one inbound blob and writes it below `output_directory`.
///
/// # Errors
///
/// Returns an error when the endpoint cannot bind, no peer connects, or the
/// received payload is invalid.
pub async fn listen(output_directory: PathBuf) -> Result<(), ProbeError> {
    let endpoint = endpoint().await?;
    let mut addr = endpoint.addr();
    if let Some(local) = local_socket(&endpoint) {
        addr.addrs.insert(TransportAddr::Ip(local));
    }
    let ticket = EndpointTicket::new(addr).to_string();
    println!("TICKET={ticket}");
    std::io::stdout()
        .flush()
        .map_err(|error| ProbeError::Io(error.to_string()))?;

    let Some(incoming) = endpoint.accept().await else {
        return Err(ProbeError::Connection("probe endpoint closed".to_owned()));
    };
    let connection = incoming
        .await
        .map_err(|error| ProbeError::Connection(error.to_string()))?;
    let mut stream = connection
        .accept_uni()
        .await
        .map_err(|error| ProbeError::Connection(error.to_string()))?;
    let metadata = read_header(&mut stream).await?;
    let path = receive_blob(&mut stream, &output_directory, &metadata)
        .await
        .map_err(|error| ProbeError::Transfer(error.to_string()))?;
    println!(
        "RECEIVED={} MIME={} SIZE={} SHA256={}",
        path.display(),
        metadata.mime_type,
        metadata.size,
        hex(&metadata.sha256),
    );
    std::io::stdout()
        .flush()
        .map_err(|error| ProbeError::Io(error.to_string()))?;
    close(&connection);
    endpoint.close().await;
    Ok(())
}

/// Sends one local file as a blob to the endpoint described by `ticket`.
///
/// # Errors
///
/// Returns an error when the ticket is invalid, the peer cannot be reached, or
/// the local file cannot be read or streamed.
pub async fn send(ticket: &str, path: PathBuf, mime_type: String) -> Result<(), ProbeError> {
    validate_mime_type(&mime_type).map_err(|error| ProbeError::Transfer(error.to_string()))?;
    let metadata =
        inspect_file(&mime_type, &path).map_err(|error| ProbeError::Transfer(error.to_string()))?;
    let parsed =
        EndpointTicket::from_str(ticket).map_err(|error| ProbeError::Ticket(error.to_string()))?;
    let endpoint = endpoint().await?;
    let _: EndpointAddr = parsed.endpoint_addr().clone();
    let connection = endpoint
        .connect(parsed.endpoint_addr().clone(), PROBE_ALPN)
        .await
        .map_err(|error| ProbeError::Connection(error.to_string()))?;
    let mut stream = connection
        .open_uni()
        .await
        .map_err(|error| ProbeError::Connection(error.to_string()))?;
    write_header(&mut stream, &metadata).await?;
    send_blob(&mut stream, &path, &metadata)
        .await
        .map_err(|error| ProbeError::Transfer(error.to_string()))?;
    stream
        .finish()
        .map_err(|error| ProbeError::Protocol(error.to_string()))?;
    println!(
        "SENT={} MIME={} SIZE={} SHA256={}",
        path.display(),
        metadata.mime_type,
        metadata.size,
        hex(&metadata.sha256),
    );
    std::io::stdout()
        .flush()
        .map_err(|error| ProbeError::Io(error.to_string()))?;
    let _ = connection.closed().await;
    endpoint.close().await;
    Ok(())
}

fn local_socket(endpoint: &Endpoint) -> Option<std::net::SocketAddr> {
    let port = endpoint
        .bound_sockets()
        .into_iter()
        .find(std::net::SocketAddr::is_ipv4)
        .map(|socket| socket.port())?;
    let probe = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    probe.connect("8.8.8.8:53").ok()?;
    let ip = probe.local_addr().ok()?.ip();
    Some(std::net::SocketAddr::new(ip, port))
}

async fn endpoint() -> Result<Endpoint, ProbeError> {
    Endpoint::builder(presets::Minimal)
        .alpns(vec![PROBE_ALPN.to_vec()])
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .map_err(|error| ProbeError::Bind(error.to_string()))
}

async fn write_header(stream: &mut SendStream, metadata: &BlobMetadata) -> Result<(), ProbeError> {
    let mime = metadata.mime_type.as_bytes();
    let mime_length = u16::try_from(mime.len())
        .map_err(|_| ProbeError::Protocol("MIME type is too long".to_owned()))?;
    stream
        .write_all(HEADER_MAGIC)
        .await
        .map_err(|error| ProbeError::Protocol(error.to_string()))?;
    stream
        .write_all(&mime_length.to_be_bytes())
        .await
        .map_err(|error| ProbeError::Protocol(error.to_string()))?;
    stream
        .write_all(mime)
        .await
        .map_err(|error| ProbeError::Protocol(error.to_string()))?;
    stream
        .write_all(&metadata.size.to_be_bytes())
        .await
        .map_err(|error| ProbeError::Protocol(error.to_string()))?;
    stream
        .write_all(&metadata.sha256)
        .await
        .map_err(|error| ProbeError::Protocol(error.to_string()))?;
    Ok(())
}

async fn read_header(stream: &mut RecvStream) -> Result<BlobMetadata, ProbeError> {
    let mut magic = [0_u8; HEADER_MAGIC.len()];
    read_exact(stream, &mut magic).await?;
    if &magic != HEADER_MAGIC {
        return Err(ProbeError::Protocol("invalid blob probe header".to_owned()));
    }
    let mut mime_length = [0_u8; 2];
    read_exact(stream, &mut mime_length).await?;
    let mime_length = usize::from(u16::from_be_bytes(mime_length));
    let mut mime = vec![0_u8; mime_length];
    read_exact(stream, &mut mime).await?;
    let mime_type = String::from_utf8(mime)
        .map_err(|error| ProbeError::Protocol(format!("invalid MIME type: {error}")))?;
    let mut size = [0_u8; 8];
    read_exact(stream, &mut size).await?;
    let mut sha256 = [0_u8; 32];
    read_exact(stream, &mut sha256).await?;
    Ok(BlobMetadata {
        mime_type,
        size: u64::from_be_bytes(size),
        sha256,
    })
}

async fn read_exact(stream: &mut RecvStream, buffer: &mut [u8]) -> Result<(), ProbeError> {
    stream
        .read_exact(buffer)
        .await
        .map_err(|error| ProbeError::Protocol(error.to_string()))
}

fn close(connection: &Connection) {
    connection.close(0_u32.into(), b"probe complete");
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}
