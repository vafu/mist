use std::env;
use std::error::Error;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use iroh::EndpointId;
use iroh::endpoint::Connection;
use mist_core::{DeviceInfo, FileStore, MistBuilder, MistRuntime, Platform, Runtime};
use qrcode::QrCode;
use qrcode::render::unicode;

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("mist: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args().skip(1);
    let command = arguments.next();
    if matches!(command.as_deref(), Some("devices" | "device" | "forget")) {
        let command = command.as_deref().unwrap();
        let id = arguments.next();
        if arguments.next().is_some()
            || (command == "devices" && id.is_some())
            || (command != "devices" && id.is_none())
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "use devices, device <endpoint-id>, or forget <endpoint-id>",
            )
            .into());
        }
        let runtime = bind_runtime().await?;
        let result: Result<(), Box<dyn Error>> = (|| {
            let devices = runtime.devices();
            if command == "forget" {
                devices.forget(id.unwrap())?;
                println!("device forgotten");
            } else {
                let peers = devices.peers()?;
                if let Some(id) = id {
                    let peer = peers
                        .into_iter()
                        .find(|peer| peer.endpoint_id == id)
                        .ok_or_else(|| {
                            io::Error::new(io::ErrorKind::NotFound, "device is not saved")
                        })?;
                    println!(
                        "{}\n{}\n{:?}",
                        peer.device.display_name, peer.endpoint_id, peer.device.platform
                    );
                } else {
                    for peer in peers {
                        println!(
                            "{}\t{}\t{:?}",
                            peer.endpoint_id, peer.device.display_name, peer.device.platform
                        );
                    }
                }
            }
            Ok(())
        })();
        runtime.shutdown().await?;
        return result;
    }
    let (bound, connection) = match command.as_deref() {
        Some("pair") => {
            if arguments.next().is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "pair does not accept arguments",
                )
                .into());
            }
            initiate_pairing().await?
        }
        Some("listen") if arguments.next().is_none() => listen().await?,
        Some("connect") => {
            let peer = arguments.next().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "connect requires an endpoint ID",
                )
            })?;
            if arguments.next().is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "connect accepts exactly one endpoint ID",
                )
                .into());
            }
            connect(peer.parse()?).await?
        }
        _ => {
            print_usage();
            return Ok(());
        }
    };

    if let Some(connection) = &connection {
        println!("connected to {}", connection.remote_id());
    }
    println!("press Ctrl-C to stop");

    tokio::select! {
        result = tokio::signal::ctrl_c() => result?,
        reason = async { match &connection { Some(connection) => connection.closed().await, None => std::future::pending().await } } => {
            println!("connection closed: {reason}");
        }
    }
    bound.shutdown().await?;
    Ok(())
}

async fn initiate_pairing()
-> Result<(std::sync::Arc<MistRuntime>, Option<Connection>), Box<dyn Error>> {
    let runtime = bind_runtime().await?;
    let pairing = runtime.pairing();
    let offer = pairing.initiate()?;
    match QrCode::new(offer.encoded.as_bytes()) {
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
    println!("pairing offer: {}", offer.encoded);
    println!("waiting for the other device");
    let peer = pairing.wait_for_peer().await?;
    println!(
        "paired with {} ({})",
        peer.device.display_name, peer.endpoint_id
    );
    let connection = runtime
        .connection(peer.endpoint_id.parse()?)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "paired peer disconnected"))?;
    Ok((runtime, Some(connection)))
}

async fn listen() -> Result<(std::sync::Arc<MistRuntime>, Option<Connection>), Box<dyn Error>> {
    let runtime = bind_runtime().await?;
    println!("endpoint: {}", runtime.endpoint_id());
    println!("waiting for peer");
    Ok((runtime, None))
}

async fn connect(
    peer: EndpointId,
) -> Result<(std::sync::Arc<MistRuntime>, Option<Connection>), Box<dyn Error>> {
    let runtime = bind_runtime().await?;
    println!("endpoint: {}", runtime.endpoint_id());
    let connection = runtime.connect(peer.into()).await?;
    Ok((runtime, Some(connection)))
}

fn print_usage() {
    eprintln!("Usage:");
    eprintln!("  mist pair");
    eprintln!("  mist listen");
    eprintln!("  mist connect <endpoint-id>");
    eprintln!("  mist devices");
    eprintln!("  mist device <endpoint-id>");
    eprintln!("  mist forget <endpoint-id>");
}

async fn bind_runtime() -> Result<Arc<MistRuntime>, Box<dyn Error>> {
    let directory = if let Some(path) = env::var_os("MIST_STATE_DIR") {
        PathBuf::from(path)
    } else if let Some(path) = env::var_os("XDG_STATE_HOME") {
        PathBuf::from(path).join("mist")
    } else {
        PathBuf::from(env::var_os("HOME").ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "HOME or MIST_STATE_DIR must be set",
            )
        })?)
        .join(".local/state/mist")
    };
    let store = Arc::new(FileStore::open(directory)?);
    Ok(MistBuilder::new(local_device_info())
        .store(store)
        .bind()
        .await?)
}

fn local_device_info() -> DeviceInfo {
    let display_name = env::var("HOSTNAME")
        .or_else(|_| env::var("COMPUTERNAME"))
        .ok()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "Mist CLI".to_owned());
    DeviceInfo {
        display_name,
        platform: Platform::Linux,
    }
}
