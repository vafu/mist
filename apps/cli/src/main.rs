use std::env;
use std::error::Error;
use std::io;

use iroh::EndpointId;
use mist_core::{DeviceClass, DeviceInfo, MistBuilder, MistRuntime, Platform};
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
    let bound = match command.as_deref() {
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

    let connection = bound
        .connection()
        .expect("the runtime installs its connection before returning");
    println!("connected to {}", connection.remote_id());
    println!("press Ctrl-C to stop");

    tokio::select! {
        result = tokio::signal::ctrl_c() => result?,
        reason = connection.closed() => {
            println!("connection closed: {reason}");
        }
    }
    bound.shutdown().await;
    Ok(())
}

async fn initiate_pairing() -> Result<std::sync::Arc<MistRuntime>, Box<dyn Error>> {
    let runtime = MistBuilder::new(local_device_info()).bind().await?;
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
    Ok(runtime)
}

async fn listen() -> Result<std::sync::Arc<MistRuntime>, Box<dyn Error>> {
    let runtime = MistBuilder::new(local_device_info()).bind().await?;
    println!("endpoint: {}", runtime.endpoint().id());
    println!("waiting for peer");
    runtime.accept().await?;
    Ok(runtime)
}

async fn connect(peer: EndpointId) -> Result<std::sync::Arc<MistRuntime>, Box<dyn Error>> {
    let runtime = MistBuilder::new(local_device_info()).bind().await?;
    println!("endpoint: {}", runtime.endpoint().id());
    runtime.connect(peer).await?;
    Ok(runtime)
}

fn print_usage() {
    eprintln!("Usage:");
    eprintln!("  mist pair");
    eprintln!("  mist listen");
    eprintln!("  mist connect <endpoint-id>");
}

fn local_device_info() -> DeviceInfo {
    let display_name = env::var("HOSTNAME")
        .or_else(|_| env::var("COMPUTERNAME"))
        .ok()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "Mist CLI".to_owned());
    DeviceInfo {
        display_name,
        device_class: DeviceClass::Desktop,
        platform: Platform::Linux,
    }
}
