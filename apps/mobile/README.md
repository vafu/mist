# Mist mobile application

This self-contained Gradle project owns the cross-platform mobile application
and its generated facade over the Rust Mist core. Android is its only configured
target during bootstrap; its Compose UI and public API live in `commonMain`.
Platform launchers and packaging live in child modules such as `android`.

The Rust API is described once with UniFFI annotations. The Gradle build pins
Gobley, generates KMP `expect` declarations plus Android `actual`
implementations, cross-compiles `libmist_core.so`, and packages it in the APK.

The current root API is generated from Rust:

```kotlin
val runtime: MistRuntime = createMistRuntime(deviceInfo, storageDirectory)
val pairing: Pairing = runtime.pairing()
val devices: Devices = runtime.devices()
```

`MistRuntime` owns the endpoint before pairing and registers authorized connections
by peer identity. The Android launcher supplies its model and platform to the
shared runtime.

The Android launcher supplies `filesDir/mist` as the persistent state directory.
Local identity and trusted peer records survive restart. Saved peers are listed
with their metadata and endpoint identity; reconnect and forget call the shared
core contracts directly. Blocking storage queries run off the UI thread. An
Android ViewModel retains the Rust handle across configuration changes and awaits
shutdown in a non-cancellable cleanup block when its lifecycle ends. The shared
composable renders the supplied core handle; it does not create domain state.

The application packages only `libmist_core` and its native dependencies. There
is no required sibling `iroh-kmp` checkout. The pinned AGP 9.3.3 supports SDK 37;
the arm64 Rust library is built with cargo-ndk and NDK 30.0.16248370.

Android pairing opens Google Code Scanner in QR-only mode and passes the
decoded `mist:pair:1:...` payload directly to the native pairing service. The
scanner is callback-driven and does not require Mist to request camera
permission. The text field remains available as a debugging fallback.
