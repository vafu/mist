use std::env;
use std::error::Error;
use std::io;

use futures_util::StreamExt;
use mist_linux::{DeviceRow, MistProxy};
use qrcode::QrCode;
use qrcode::render::unicode;
use zbus::Connection;

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("mist: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn Error>> {
    let connection = Connection::session().await?;
    let mist = MistProxy::new(&connection).await?;
    let mut arguments = env::args().skip(1);
    match arguments.next().as_deref() {
        Some("status") if arguments.next().is_none() => status(&mist).await?,
        Some("devices") if arguments.next().is_none() => devices(&mist).await?,
        Some("pair") if arguments.next().is_none() => pair(&mist).await?,
        Some("pair-cancel") if arguments.next().is_none() => mist.cancel_pairing().await?,
        Some("select") => select(&mist, arguments, true).await?,
        Some("unselect") => select(&mist, arguments, false).await?,
        Some("connect") => device_command(&mist, arguments, true).await?,
        Some("disconnect") => device_command(&mist, arguments, false).await?,
        Some("forget") => forget(&mist, arguments).await?,
        Some("notify") => notify(&mist, arguments).await?,
        Some("clipboard") => clipboard(&mist, arguments).await?,
        Some("clipboard-file") => clipboard_file(&mist, arguments).await?,
        Some("share") => share_file(&mist, arguments).await?,
        Some("watch") if arguments.next().is_none() => watch(&mist).await?,
        Some("shutdown") if arguments.next().is_none() => mist.shutdown().await?,
        _ => print_usage(),
    }
    Ok(())
}

async fn status(proxy: &MistProxy<'_>) -> zbus::Result<()> {
    println!("endpoint: {}", proxy.endpoint_id().await?);
    println!("connected: {}", proxy.connected().await?);
    println!("devices: {}", proxy.devices().await?.len());
    Ok(())
}

async fn devices(proxy: &MistProxy<'_>) -> zbus::Result<()> {
    let devices = proxy.devices().await?;
    if devices.is_empty() {
        println!("No paired devices");
        return Ok(());
    }
    for device in devices {
        print_device(&device);
    }
    Ok(())
}

async fn pair(proxy: &MistProxy<'_>) -> Result<(), Box<dyn Error>> {
    let offer = proxy.start_pairing().await?;
    match QrCode::new(offer.as_bytes()) {
        Ok(code) => {
            let image = code
                .render::<unicode::Dense1x2>()
                .dark_color(unicode::Dense1x2::Light)
                .light_color(unicode::Dense1x2::Dark)
                .build();
            println!("Scan to pair:\n{image}");
        }
        Err(error) => eprintln!("could not render pairing QR code: {error}"),
    }
    println!("pairing offer: {offer}");
    println!("mistd is waiting for the peer in the background");
    Ok(())
}

async fn select(
    proxy: &MistProxy<'_>,
    mut arguments: impl Iterator<Item = String>,
    selected: bool,
) -> Result<(), Box<dyn Error>> {
    let endpoint_id = one_argument(&mut arguments, "device endpoint ID")?;
    proxy.select_device(&endpoint_id, selected).await?;
    Ok(())
}

async fn device_command(
    proxy: &MistProxy<'_>,
    mut arguments: impl Iterator<Item = String>,
    connect: bool,
) -> Result<(), Box<dyn Error>> {
    let endpoint_id = one_argument(&mut arguments, "device endpoint ID")?;
    if connect {
        proxy.connect_device(&endpoint_id).await?;
    } else {
        proxy.disconnect_device(&endpoint_id).await?;
    }
    Ok(())
}

async fn forget(
    proxy: &MistProxy<'_>,
    mut arguments: impl Iterator<Item = String>,
) -> Result<(), Box<dyn Error>> {
    let endpoint_id = one_argument(&mut arguments, "device endpoint ID")?;
    proxy.forget_device(&endpoint_id).await?;
    Ok(())
}

async fn notify(
    proxy: &MistProxy<'_>,
    arguments: impl Iterator<Item = String>,
) -> Result<(), Box<dyn Error>> {
    let (mut arguments, targets) = split_options(arguments.collect())?;
    let title = arguments
        .first()
        .cloned()
        .ok_or_else(|| invalid_input("notify requires a title"))?;
    let mut rest = arguments.split_off(1);
    let mut image_path = String::new();
    if let Some(index) = rest.iter().position(|argument| argument == "--image")
        && index + 1 < rest.len()
    {
        image_path = rest.remove(index + 1);
        rest.remove(index);
    }
    let body = rest.join(" ");
    proxy
        .push_notification(&title, &body, &image_path, targets)
        .await?;
    Ok(())
}

async fn clipboard(
    proxy: &MistProxy<'_>,
    arguments: impl Iterator<Item = String>,
) -> Result<(), Box<dyn Error>> {
    let (arguments, targets) = split_options(arguments.collect())?;
    let text = arguments
        .first()
        .cloned()
        .ok_or_else(|| invalid_input("clipboard requires text"))?;
    proxy.push_clipboard_text(&text, targets).await?;
    Ok(())
}

async fn clipboard_file(
    proxy: &MistProxy<'_>,
    arguments: impl Iterator<Item = String>,
) -> Result<(), Box<dyn Error>> {
    let (arguments, targets) = split_options(arguments.collect())?;
    let path = arguments
        .first()
        .cloned()
        .ok_or_else(|| invalid_input("clipboard-file requires a path"))?;
    let mime_type = arguments
        .get(1)
        .cloned()
        .unwrap_or_else(|| "application/octet-stream".to_owned());
    proxy
        .push_clipboard_blob(&path, &mime_type, targets)
        .await?;
    Ok(())
}

async fn share_file(
    proxy: &MistProxy<'_>,
    arguments: impl Iterator<Item = String>,
) -> Result<(), Box<dyn Error>> {
    let (arguments, targets) = split_options(arguments.collect())?;
    let path = arguments
        .first()
        .cloned()
        .ok_or_else(|| invalid_input("share requires a path"))?;
    proxy.share_file(&path, targets).await?;
    Ok(())
}

/// Splits `--targets=a,b` / `--targets a,b` options from positional arguments.
fn split_options(arguments: Vec<String>) -> Result<(Vec<String>, Vec<String>), io::Error> {
    let mut positional = Vec::new();
    let mut targets = Vec::new();
    let mut remaining = arguments.into_iter();
    while let Some(argument) = remaining.next() {
        if let Some(value) = argument.strip_prefix("--targets=") {
            targets.extend(split_targets(value));
        } else if argument == "--targets" {
            let value = remaining
                .next()
                .ok_or_else(|| invalid_input("--targets requires an endpoint ID list"))?;
            targets.extend(split_targets(&value));
        } else {
            positional.push(argument);
        }
    }
    Ok((positional, targets))
}

fn split_targets(value: &str) -> impl Iterator<Item = String> + '_ {
    value
        .split(',')
        .map(str::trim)
        .filter(|target| !target.is_empty())
        .map(str::to_owned)
}

async fn watch(proxy: &MistProxy<'_>) -> zbus::Result<()> {
    let mut device_changes = proxy.receive_device_changed().await?;
    let mut notifications = proxy.receive_notification_received().await?;
    let mut pairing_failures = proxy.receive_pairing_failed().await?;
    let mut clipboard = proxy.receive_clipboard_received().await?;
    loop {
        tokio::select! {
            Some(signal) = device_changes.next() => {
                let args = signal.args()?;
                println!("device changed: {}", args.endpoint_id());
            }
            Some(signal) = notifications.next() => {
                let args = signal.args()?;
                println!("notification from {}: {} — {}", args.endpoint_id(), args.title(), args.body());
            }
            Some(signal) = clipboard.next() => {
                let args = signal.args()?;
                if args.mime_type().starts_with("text/") {
                    println!("clipboard from {}: {}", args.endpoint_id(), args.text());
                } else {
                    println!(
                        "clipboard from {}: {} ({})",
                        args.endpoint_id(),
                        args.mime_type(),
                        args.path(),
                    );
                }
            }
            Some(signal) = pairing_failures.next() => {
                let args = signal.args()?;
                eprintln!("pairing failed: {}", args.reason());
            }
            else => return Ok(()),
        }
    }
}

fn print_device(device: &DeviceRow) {
    let (endpoint_id, name, class, platform, status, transport, selected) = device;
    println!(
        "{name}\n  id: {endpoint_id}\n  {class}/{platform} · {status} via {transport} · selected={selected}"
    );
}

fn one_argument(
    arguments: &mut impl Iterator<Item = String>,
    name: &str,
) -> Result<String, io::Error> {
    let value = arguments
        .next()
        .ok_or_else(|| invalid_input(format!("missing {name}")))?;
    if arguments.next().is_some() {
        return Err(invalid_input("too many arguments"));
    }
    Ok(value)
}

fn invalid_input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn print_usage() {
    eprintln!("Usage:");
    eprintln!("  mist status");
    eprintln!("  mist devices");
    eprintln!("  mist pair | pair-cancel");
    eprintln!("  mist select|unselect <endpoint-id>");
    eprintln!("  mist connect|disconnect|forget <endpoint-id>");
    eprintln!("  mist notify <title> [body] [--targets <id,...>]");
    eprintln!("  mist clipboard <text> [--targets <id,...>]");
    eprintln!("  mist clipboard-file <path> [mime-type] [--targets <id,...>]");
    eprintln!("  mist share <path> [--targets <id,...>]");
    eprintln!("  mist watch");
    eprintln!("  mist shutdown");
}

#[cfg(test)]
mod test {
    use super::split_options;

    #[test]
    fn targets_accept_comma_separated_and_repeated_forms() {
        let (positional, targets) = split_options(vec![
            "--targets".to_owned(),
            "a,b".to_owned(),
            "body".to_owned(),
            "--targets=c".to_owned(),
        ])
        .unwrap();
        assert_eq!(positional, vec!["body".to_owned()]);
        assert_eq!(
            targets,
            vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]
        );
    }

    #[test]
    fn no_targets_means_selected_devices() {
        let (positional, targets) = split_options(vec!["hello".to_owned()]).unwrap();
        assert_eq!(positional, vec!["hello".to_owned()]);
        assert!(targets.is_empty());
    }
}
