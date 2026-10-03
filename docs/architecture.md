# Architecture

Mist is a native Rust application with platform bindings around the same core.
Iroh is the networking foundation, not an interchangeable adapter.

## Current workspace

```text
core                Shared Rust runtime and feature contracts (`mist-core`)
core/proto          Versioned Protobuf schemas for the Mist wire protocol
apps/cli            Fully Rust Linux command-line host
apps/mobile         KMP mobile application and native binding contract
apps/mobile/android Thin Android application launcher and package
```

`apps/mobile` is a self-contained Gradle project. The repository root and the
native core have no Gradle build files.

`mist-core` creates and owns `MistRuntime`, the root of one Mist process. The
runtime exists before pairing, always contains its Iroh endpoint, and has a
one-time connection slot. Pairing installs exactly one stable connection into
that slot. Iroh may migrate the connection between IP, relay, and custom
transports without replacing it.

Custom transports and address lookups are installed on `MistBuilder` before
the endpoint binds. This is the native extension point that the KMP bridge will
use for the existing `iroh-kmp` Bluetooth transport.

## Generated mobile boundary

Rust is the source of truth for the mobile API. UniFFI metadata and Gobley
generate `expect` declarations in `commonMain` and Android `actual`
implementations. A generated `MistRuntime` is a proxy to the exact same
`Arc<MistRuntime>` allocated in Rust, not a Kotlin-owned duplicate.

```text
MistRuntime
├── Iroh endpoint
├── one stable connection slot
└── Pairing
```

`Pairing` is a Rust trait exported as a generated Kotlin interface. The CLI
creates an opaque invitation containing an Iroh endpoint ticket and a random
single-use token. Android connects from that ticket and proves
possession of the token before either side exchanges device metadata or its
session capability snapshot. The candidate connection is installed into the
runtime only after that handshake succeeds.

Pairing is event-driven: offer state is published reactively, incoming
handshakes are handled concurrently, and cancellation or connection closure
wakes the relevant operation directly. Timers, polling, and artificial delays
are not part of protocol control flow.

Iroh authenticates and encrypts the transport endpoint; Mist's invitation token
authorizes the previously unknown endpoint to become the paired peer. Invalid
connections do not consume the invitation. Trusted-device persistence and
reconnection are not implemented yet.

Gobley 0.3.7's Gradle plugin predates AGP 9's KMP Android plugin. The mobile
build therefore invokes the pinned Gobley generator directly and wires its
generated source directories into KMP. This remains generated code and uses the
same UniFFI ABI; the task can be replaced by the plugin when AGP 9 support is
released.

Platform capabilities will be Rust traits defined beside the core feature that
consumes them. Linux hosts implement them in Rust. Mobile hosts implement their
generated Kotlin foreign-trait interfaces. Feature-specific traits stay small;
there is no monolithic platform provider.

Wire schemas live under `core/proto`. `prost` generates private Rust transport
types which are explicitly converted to Mist domain types; generated Protobuf
types never cross the public Rust or UniFFI APIs. Pairing authorization precedes
the exchange of device metadata and the initial session capability snapshot.
Capabilities are negotiated again on every connection and may be updated while
the session is live.

## Growth rules

- Feature protocol and state live in Rust.
- Every side effect or host dependency entering the core is represented by a
  narrow Rust trait.
- Shared mobile application code lives in `apps/mobile`; platform launchers and
  packaging live below it.
- Android owns Android APIs, permissions, UI, and lifecycle integration.
- Linux platform integration stays in the CLI crate until another Linux host
  needs to reuse it.
- New crates are introduced only for a real dependency or lifecycle boundary.
- The KMP facade packages `libmist_core`; generated Kotlin objects retain Rust
  objects by native handle.
