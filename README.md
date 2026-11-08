# Mist (Міст)

Mist is an Iroh-based bridge between devices and an alternative to KDE
Connect. The shared runtime and native applications are written in Rust.
Platform bindings reuse the same Rust-owned Iroh endpoint and connection.

The bootstrap currently contains:

- `core/app-contract`, the dependency-light public Rust/UniFFI API.
- `core`, the private Rust runtime implementation, which owns Iroh, SQLite
  persistence, the multi-device registry, and feature implementations.
- `apps/cli`, the Rust `mistd` Linux daemon and its thin D-Bus client, `mist`.
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
uses Gobley to generate Kotlin bindings from both Rust crates. Set
`ANDROID_HOME` when that repository does not have its own `local.properties`.
The Android foreground service runs in a dedicated `:mist` process; the
activity is a Binder-backed controller and does not own native state.

Run the Linux daemon, then start pairing from the D-Bus CLI. `mist pair`
returns immediately after printing both a terminal QR code and the opaque
invitation text; `mistd` continues accepting it in the background:

```sh
cargo run -p mist-cli --bin mistd
cargo run -p mist-cli --bin mist -- pair
```

To pair with Android, scan the QR code in the app or paste the invitation into
its debug field. Its authorization token is single-use and remains active until
it is consumed, replaced, cancelled, or the runtime shuts down.

The Linux D-Bus service is `io.github.vafu.Mist`, at
`/io/github/vafu/Mist`, with interface `io.github.vafu.Mist1`. Packaging files
for D-Bus and systemd user activation live under `apps/cli/dbus-1` and
`apps/cli/systemd/user`. Useful commands include:

```sh
mist status
mist devices
mist select <endpoint-id>
mist connect <endpoint-id>
mist disconnect <endpoint-id>
mist forget <endpoint-id>
mist notify "Title" "Body"
mist watch
```

On Linux, `mistd` monitors calls to the standard desktop notification service
and forwards them to selected devices. Notifications received from a peer are
published through that same desktop service. On Android, a
`NotificationListenerService` forwards local notifications into the Rust
channel, while remote notifications are posted through Android's native
notification manager. Mist filters its own Android notifications to prevent
echo loops.

Rust persists the Iroh identity, trusted devices, selection state, and internal
address records in SQLite. Linux uses `$XDG_STATE_HOME/mist` (falling back to
`$HOME/.local/state/mist`); Android uses its app-private files directory.

See [the architecture notes](docs/architecture.md) for the native/KMP ownership
boundary.
