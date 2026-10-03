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
val runtime: MistRuntime = createMistRuntime(deviceInfo)
val pairing: Pairing = runtime.pairing()
```

`MistRuntime` owns the endpoint before pairing and retains the stable connection
after pairing. The Android launcher supplies its model, device class, and
platform to the shared runtime. Future platform providers and Iroh custom
transports will be additional parameters of the runtime composition factory.

Android pairing opens Google Code Scanner in QR-only mode and passes the
decoded `mist:pair:1:...` payload directly to the native pairing service. The
scanner is callback-driven and does not require Mist to request camera
permission. The text field remains available as a debugging fallback.

Feature bindings for notifications, clipboard, and file transfer will share
the same native runtime handle. Generic Iroh operations will continue to use
iroh-kmp wrappers rather than being duplicated in Mist's API.
