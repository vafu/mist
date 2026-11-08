# Architecture

Mist is a native Rust application with platform bindings around one shared
core. Iroh is the networking foundation, not an interchangeable adapter.

## Dependency boundaries

```text
apps/cli ───────────────┐
apps/mobile service ────┼── mist-app-contract
                        │          ▲
                        └── mist-core (private runtime implementation)
                                   ├── Iroh endpoint and sessions
                                   ├── SQLite state store
                                   └── protobuf wire protocol
```

`core/app-contract` is the complete app-facing API. It contains only
platform-neutral records, errors, and traits: `MistApp`, `DeviceManager`,
`Pairing`, `NotificationChannel`, and their reactive event streams. It does not
depend on Iroh, Tokio, SQLite, prost, zbus, or Android.

`core` implements the contract. Its concrete runtime, device registry,
connections, storage traits, address lookup, executor, and protobuf types are
private. Hosts enter through `mist_core::host::MistFactory`, configure optional
Iroh custom transports/address lookups before bind, and then retain only
`Arc<dyn MistApp>`. Raw addresses are never exposed through the app contract.

UniFFI/Gobley generates two Kotlin namespaces from the linked native library:
`io.github.vafu.mist.core` for the app contract and
`io.github.vafu.mist.host` for the small construction seam. This keeps generated
runtime support symbols separate while allowing `createMistApp` to return the
generated contract object.

## Identity and persistence

Every production host supplies an app-private state directory. Rust owns a
SQLite database there and persists:

- the 32-byte Iroh secret key that defines the local endpoint identity;
- trusted endpoint IDs and authenticated device metadata;
- each device's user-controlled selected state;
- transport addresses with their provenance.

Live connection state and the currently selected transport are reconstructed
and never stored. Existing malformed identity or device data fails startup;
Mist never silently replaces a corrupt identity.

The stored address lookup implements Iroh's native `AddressLookup`. Address
records use add/remove operations and retain provenance (`pairing_offer` or
`authenticated_peer`). Pairing tickets bootstrap the first connection, while
authenticated session hellos and subsequent address snapshots update future
reachability. Iroh publishes local address changes into a watch channel; Mist
forwards those changes over the existing authenticated `mist/1` session. There
is no address ALPN, polling loop, or refresh timer.

## Pairing and sessions

An invitation contains an Iroh endpoint ticket and a random single-use token.
The accepting peer proves possession before either side accepts device metadata,
capabilities, or addresses. Iroh authenticates and encrypts the transport;
Mist's token authorizes the previously unknown endpoint to enter the trusted
device store.

Each trusted device has independent metadata, selected state, connection state,
generation, and Iroh connection. Replacing a connection increments its
generation so an old close event cannot mark the replacement offline. Iroh can
migrate a connection among direct IP, relay, and custom transports; path events
update `ManagedDevice` reactively.

`DeviceManager` exposes only user decisions: list/get, subscribe, select,
connect, disconnect, and forget. Forget closes any live connection and removes
trust, metadata, and addresses through SQLite foreign-key cleanup.

## Application processes

Linux uses a native daemon and a thin D-Bus client:

```text
mist CLI ── io.github.vafu.Mist1 ── mistd ── MistApp ── Iroh peers
                                      │
                                      └── org.freedesktop.Notifications
```

`mistd` stores data under `$XDG_STATE_HOME/mist` (or
`$HOME/.local/state/mist`), publishes device/connection properties and signals,
monitors freedesktop notifications, and displays remote notifications through
the standard notification service.

Android deliberately has two application processes:

```text
main process                         :mist process
MainActivity/Compose                MistRuntimeService (foreground)
       │                                  ├── Rust MistApp owner
       └── AndroidMistController ─Binder──├── notification receive loop
                                          └── NotificationListenerService
```

Native handles never cross Binder. The service translates contract values into
`Bundle` snapshots and asynchronous result callbacks. The shared KMP UI depends
only on `MistController`; a future iOS host can implement that same controller
in-process while retaining the same Rust app contract. Closing or recreating an
activity does not restart the endpoint.

## Growth rules

- Feature protocol and state live in Rust.
- App-visible types and operations start in `mist-app-contract` and are
  generated for Kotlin.
- Construction and Iroh host extensions stay in `mist-core::host`.
- Every platform dependency entering the runtime uses a narrow trait.
- Addresses and protobuf types stay private.
- Production coordination is reactive: no sleeps, polling intervals, or
  timer-driven synchronization.
- Shared mobile UI lives in `apps/mobile`; Android lifecycle and Binder code
  lives only in `apps/mobile/android`.
