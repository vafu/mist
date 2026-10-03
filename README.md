# Mist (Міст)

Mist is an Iroh-based bridge between devices and an alternative to KDE
Connect. The shared runtime and native applications are written in Rust.
Platform bindings reuse the same Rust-owned Iroh endpoint and connection.

The bootstrap currently contains:

- `core`, the Rust `mist-core` crate, which owns one Iroh endpoint and one
  stable peer connection plus feature interfaces such as `Pairing`.
- `apps/cli`, the fully Rust `mist` CLI.
- `apps/mobile`, the shared KMP mobile application and generated UniFFI API.
- `apps/mobile/android`, the thin Android launcher and package.

## Build and test

```sh
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all --check
```

Build the Android target and its KMP host tests with:

```sh
cd apps/mobile
./gradlew check :android:assembleDebug
```

The mobile build consumes `../../../iroh-kmp` as an included source build and
uses Gobley to generate Kotlin bindings from the Rust API. Set `ANDROID_HOME`
when that repository does not have its own `local.properties`.

Start pairing from the CLI. It prints both a terminal QR code and the opaque
invitation text:

```sh
cargo run -p mist-cli -- pair
```

To pair with Android, scan the CLI's QR code with any QR scanner and paste the
decoded invitation into the mobile app. Its authorization token is single-use
and remains active until it is consumed, replaced, cancelled, or the runtime
shuts down.

See [the architecture notes](docs/architecture.md) for the native/KMP ownership
boundary.
