use std::error::Error;
use std::path::PathBuf;

use mist_core::probe::{ProbeError, listen, send};

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("mist-blob-probe: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args().skip(1);
    match arguments.next().as_deref() {
        Some("listen") => {
            let directory = arguments.next().ok_or(ProbeError::Usage)?;
            listen(PathBuf::from(directory)).await?;
        }
        Some("send") => {
            let ticket = arguments.next().ok_or(ProbeError::Usage)?;
            let path = arguments.next().ok_or(ProbeError::Usage)?;
            let mime_type = arguments
                .next()
                .unwrap_or_else(|| "application/octet-stream".to_owned());
            send(&ticket, PathBuf::from(path), mime_type).await?;
        }
        _ => return Err(ProbeError::Usage.into()),
    }
    Ok(())
}
