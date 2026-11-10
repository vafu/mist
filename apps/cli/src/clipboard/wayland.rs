use std::fmt;
use std::io::Read;
use std::path::PathBuf;

use mist_app_contract::ClipboardContent;
use wl_clipboard_rs::copy::{self, MimeSource, MimeType as CopyMimeType, Options, Source};
use wl_clipboard_rs::paste::{MimeType as PasteMimeType, Seat};
use wl_clipboard_rs::watch::{ClipboardEvent, ClipboardType as WatchClipboard, Watcher};

use super::{LocalClipboardError, LocalClipboardProvider, LocalClipboardWatcher};

/// Maximum clipboard payload accepted from or written to the local clipboard.
const MAX_CLIPBOARD_BYTES: usize = 32 * 1024 * 1024;

/// MIME types offered for binary clipboard content, in preference order.
const BINARY_MIME_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/webp", "image/gif"];

/// Wayland clipboard integration backed by `wl-clipboard-rs`.
pub struct WaylandClipboard {
    blob_directory: PathBuf,
}

impl WaylandClipboard {
    /// Creates a provider that stores observed binary content in `blob_directory`.
    #[must_use]
    pub fn new(blob_directory: impl Into<PathBuf>) -> Self {
        Self {
            blob_directory: blob_directory.into(),
        }
    }
}

impl fmt::Debug for WaylandClipboard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WaylandClipboard")
            .field("blob_directory", &self.blob_directory)
            .finish()
    }
}

impl LocalClipboardProvider for WaylandClipboard {
    fn watch(&self) -> Result<Box<dyn LocalClipboardWatcher>, LocalClipboardError> {
        let watcher =
            Watcher::new(WatchClipboard::Regular, Seat::Unspecified).map_err(unavailable)?;
        Ok(Box::new(WaylandClipboardWatcher {
            watcher,
            blob_directory: self.blob_directory.clone(),
        }))
    }

    fn apply(&self, content: &ClipboardContent) -> Result<(), LocalClipboardError> {
        let sources = match content {
            ClipboardContent::Text { text } => vec![MimeSource {
                source: Source::Bytes(text.as_bytes().to_vec().into()),
                mime_type: CopyMimeType::Specific("text/plain;charset=utf-8".to_owned()),
            }],
            ClipboardContent::Blob { mime_type, path } => {
                let metadata =
                    std::fs::metadata(path).map_err(|error| LocalClipboardError::Read {
                        reason: error.to_string(),
                    })?;
                if metadata.len() > MAX_CLIPBOARD_BYTES as u64 {
                    return Err(LocalClipboardError::TooLarge);
                }
                let bytes = std::fs::read(path).map_err(|error| LocalClipboardError::Read {
                    reason: error.to_string(),
                })?;
                image_sources(mime_type, &bytes)
            }
        };

        let mut options = Options::new();
        options.clipboard(copy::ClipboardType::Regular);
        copy::copy_multi(options, sources).map_err(|error| LocalClipboardError::Write {
            reason: error.to_string(),
        })
    }
}

/// Publishes the image under its original MIME type plus PNG.
///
/// Paste targets commonly request `image/png` specifically; offering only the
/// original format (usually `image/jpeg`) makes an otherwise valid image appear
/// missing to those clients.
fn image_sources(mime_type: &str, bytes: &[u8]) -> Vec<MimeSource> {
    let mut sources = vec![MimeSource {
        source: Source::Bytes(bytes.to_owned().into()),
        mime_type: CopyMimeType::Specific(mime_type.to_owned()),
    }];
    if mime_type != "image/png"
        && let Ok(image) = image::load_from_memory(bytes)
    {
        let mut png = Vec::new();
        if image
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .is_ok()
            && png.len() <= MAX_CLIPBOARD_BYTES
        {
            sources.push(MimeSource {
                source: Source::Bytes(png.into()),
                mime_type: CopyMimeType::Specific("image/png".to_owned()),
            });
        }
    }
    sources
}

struct WaylandClipboardWatcher {
    watcher: Watcher,
    blob_directory: PathBuf,
}

impl LocalClipboardWatcher for WaylandClipboardWatcher {
    fn next(&mut self) -> Result<Option<ClipboardContent>, LocalClipboardError> {
        let event = self
            .watcher
            .next_event()
            .map_err(|error| LocalClipboardError::Read {
                reason: error.to_string(),
            })?;
        let Some(event) = event else {
            return Ok(None);
        };
        let ClipboardEvent::Changed {
            mime_types,
            mut offer,
            ..
        } = event
        else {
            return Ok(None);
        };
        let Some(mime_type) = preferred_mime_type(&mime_types) else {
            return Ok(None);
        };
        let mut pipe = offer
            .receive(&mime_type)
            .map_err(|error| LocalClipboardError::Read {
                reason: error.to_string(),
            })?;
        let bytes = read_bounded(&mut pipe)?;

        if is_text_mime(&mime_type) {
            let text = String::from_utf8(bytes).map_err(|_| LocalClipboardError::InvalidText)?;
            if text.is_empty() {
                return Ok(None);
            }
            return Ok(Some(ClipboardContent::Text { text }));
        }

        std::fs::create_dir_all(&self.blob_directory).map_err(|error| {
            LocalClipboardError::Read {
                reason: error.to_string(),
            }
        })?;
        let path = self.blob_directory.join(format!(
            "local-{}.{}",
            unique_suffix(),
            extension(&mime_type)
        ));
        std::fs::write(&path, &bytes).map_err(|error| LocalClipboardError::Read {
            reason: error.to_string(),
        })?;
        Ok(Some(ClipboardContent::Blob {
            mime_type,
            path: path.to_string_lossy().into_owned(),
        }))
    }
}

fn preferred_mime_type(mime_types: &[String]) -> Option<String> {
    BINARY_MIME_TYPES
        .iter()
        .find(|candidate| mime_types.iter().any(|offered| offered == *candidate))
        .map(|candidate| (*candidate).to_owned())
        .or_else(|| {
            wl_clipboard_rs::paste::select_mime_type(mime_types.to_vec(), PasteMimeType::Text)
        })
}

fn is_text_mime(mime_type: &str) -> bool {
    wl_clipboard_rs::utils::is_text(mime_type)
}

fn read_bounded(pipe: &mut impl Read) -> Result<Vec<u8>, LocalClipboardError> {
    let mut bytes = Vec::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = pipe
            .read(&mut buffer)
            .map_err(|error| LocalClipboardError::Read {
                reason: error.to_string(),
            })?;
        if read == 0 {
            break;
        }
        if bytes.len() + read > MAX_CLIPBOARD_BYTES {
            return Err(LocalClipboardError::TooLarge);
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    Ok(bytes)
}

/// File extension for a clipboard MIME type.
fn extension(mime_type: &str) -> &'static str {
    match mime_type {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "bin",
    }
}

fn unique_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!("{nanos:x}")
}

fn unavailable(error: impl fmt::Display) -> LocalClipboardError {
    LocalClipboardError::Unavailable {
        reason: error.to_string(),
    }
}
