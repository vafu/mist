# Mist mobile application

This self-contained Gradle project owns the shared KMP UI and platform mobile
hosts. Android is currently the only configured target.

Rust defines `mist-app-contract`; UniFFI metadata and the pinned Gobley generator
produce KMP `expect` declarations and Android `actual` implementations. The
small `mist-core` host namespace exports `createMistApp(storageDirectory,
deviceInfo)`. Rust owns the SQLite database, stable Iroh identity, trusted
devices, addresses, and live sessions.

The shared Compose UI consumes `MistController`, not a native runtime handle.
On Android, `AndroidMistController` binds to `MistRuntimeService` in the
dedicated `:mist` process. That foreground service owns the only Rust `MistApp`
allocation and sends process-safe snapshots and asynchronous results over AIDL.
Activities can be destroyed and reattached without replacing the endpoint. A
future iOS host can provide an in-process controller with the same shared UI.

The Android notification listener runs in the service process and forwards
platform notifications through Binder to the Rust `NotificationChannel`.
Remote notifications are consumed and displayed by `MistRuntimeService`.
Notification posting permission and notification-listener access remain
separate user permissions.

Pairing uses Google Code Scanner in QR-only mode and passes the opaque
`mist:pair:1:...` payload to the service. The text field is retained as an
emulator/debug fallback. Device state is pushed reactively; the UI can select,
connect, disconnect, and forget trusted devices.

Build with:

```sh
ANDROID_HOME=/path/to/Android/Sdk ./gradlew check :android:assembleDebug
```
