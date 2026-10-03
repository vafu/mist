use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let proto_root = "proto";
    let protos = [
        "proto/mist/v1/device.proto",
        "proto/mist/v1/pairing.proto",
        "proto/mist/v1/session.proto",
        "proto/mist/v1/control.proto",
    ];

    let mut config = prost_build::Config::new();
    config.protoc_executable(protoc_bin_vendored::protoc_bin_path()?);
    config.compile_protos(&protos, &[proto_root])?;

    Ok(())
}
