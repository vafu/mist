# Mist (Міст)

Mist is an Iroh-based bridge between devices and an alternative to KDE
Connect. The shared runtime and native applications are written in Rust.
Platform bindings reuse the same Rust-owned Iroh endpoint and peer connections.

The bootstrap currently contains:

- `core`, the Rust `mist-core` crate: trait-backed runtime, concurrent peer
  connections, persistent identities and trusted devices, pairing and reconnect.
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

The mobile build uses Gobley to generate Kotlin bindings from the Rust API and
packages the arm64 Android core. Set `ANDROID_HOME` to your SDK directory.
Install the `aarch64-linux-android` Rust target, `cargo-ndk`, and NDK
`30.0.16248370` for native builds. SDK 37 and Java 21 are used by the pinned build.

Start pairing from the CLI. It prints both a terminal QR code and the opaque
invitation text:

```sh
cargo run -p mist-cli -- pair
```

To pair with Android, use the app's QR scanner or paste the decoded invitation
into its text field. The authorization token is single-use
and remains active until it is consumed, replaced, cancelled, or the runtime
shuts down.

## Saved devices

The CLI keeps its local identity and trusted peers under
`$XDG_STATE_HOME/mist` (or `~/.local/state/mist`). Override this with
`MIST_STATE_DIR`. Android uses its private `files/mist` directory. Each directory
has one active writer; shutdown releases the instance, and dropping its handles
releases the storage lock.

```sh
cargo run -p mist-cli -- devices
cargo run -p mist-cli -- device <endpoint-id>
cargo run -p mist-cli -- forget <endpoint-id>
cargo run -p mist-cli -- listen
cargo run -p mist-cli -- connect <trusted-endpoint-id>
```

Saved peers remain available while offline. After restart, either side can
initiate reconnect without another invitation; both installations must retain
trust. `listen` keeps the core running to accept trusted peers. The Android UI
lists saved peers and provides reconnect, details and forget controls.
Forgetting removes local trust and closes the connection; the peer needs a fresh
invitation to pair again. Automatic reconnect retry is not implemented.

See [the architecture notes](docs/architecture.md) for the native/KMP ownership
boundary.
