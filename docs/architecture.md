# Architecture

Mist has one shared Rust core for Linux and Android. Apps supply host inputs and
present core results; they do not own pairing, trust, connection or storage rules.
Iroh is the networking foundation.

## Workspace

```text
core/                 Shared runtime, device storage, pairing and protocol
core/proto/           Private versioned Protobuf wire schemas
apps/cli/             Rust command-line adapter
apps/mobile/          Shared Compose UI and generated UniFFI/Gobley bindings
apps/mobile/android/  Android launcher, private state path and native packaging
```

## Contracts and ownership

Core behavioral components have trait contracts. Module roots define contracts,
domain values, errors and intentional reexports; sibling files implement them.

```text
MistRuntime implements Runtime
├── one local Iroh endpoint
├── Arc<dyn Connections> → active connections keyed by remote endpoint ID
├── Arc<dyn Pairing> → invitation state and authorization
├── Arc<dyn Devices> → saved-device operations
└── instance cancellation and tracked tasks on core-owned execution

DeviceStore
├── local Iroh identity
└── saved Peer records (endpoint ID, display name, platform)
```

There is no mixed local-state/single-peer `Session`. Pairing receives local
metadata explicitly and depends on the connection contract. No executor getter,
device snapshot/subscription layer, duplicate trusted-device record, or separate
mobile domain API is needed.

## Connections and pairing

Either peer may initiate, and an instance can connect to multiple peers at once.
One core accept loop reads the initial control message and dispatches it to
pairing authorization or trusted reconnect. `Pairing::wait_for_peer` waits for
the invitation's result; it does not accept endpoint connections itself.

An invitation contains an Iroh endpoint ticket and a random single-use token.
Invalid tokens do not consume it. Valid authorization consumes it before
metadata exchange and persistence; later failure requires a new invitation.
Creating another invitation or cancelling invalidates the previous invitation.

Iroh authenticates the remote endpoint identity. Mist's token authorizes a new
peer. Pairing stores the peer locally before reporting success. There is no
distributed storage transaction: a failure can leave one installation with trust
that the other has not retained. A new invitation can repair that partial state.

Reconnect requires saved trust, checked again when registering the connection.
Trust removal and connection registration are serialized so an in-flight
reconnect using an old trust read cannot resurrect a forgotten peer. Forgetting
closes that peer's connection and invalidates invitations; it does not promise
to remove records on an offline remote installation.

Simultaneous reconnects use a deterministic rank derived from the connection's
TLS exporter, which produces the same bytes at both ends. Both peers retain the
same winner and close duplicates. Re-pairing refreshes a record without replacing
a working connection. Closed transports are not reported as connected and can
be replaced without disturbing other peers.

## Identity and storage

`DeviceStore` is the persistence contract. `FileStore` is shared Rust code used
on Linux and Android. It stores the local key and peer records in an installation
directory, holds an exclusive OS writer lock, and uses atomic replacement plus
file/directory synchronization for updates. Corrupt/unreadable state is an error,
not an instruction to regenerate identity or discard trusted peers.

`MistBuilder::store` supplies persistent or test storage. Without it, the builder
uses an isolated `MemoryStore` for ephemeral/testing use. An explicitly supplied
key must match the store's identity. The exported default constructor takes local
metadata and a storage directory and uses the same builder and file store.

## Execution and lifecycle

Construction and async operations run on a core-owned Tokio runtime. They work
without a host Tokio reactor and survive a caller runtime being dropped. There
is one public runtime error contract for Rust and generated calls.

Shutdown rejects new work, invalidates invitations, cancels and waits for
instance-owned tasks, and closes every peer connection and the endpoint. Repeated
shutdown is safe and does not stop other instances. Dropping the last handle
also starts cleanup, including when construction finishes after its caller has
cancelled. Explicit async shutdown provides completion/error reporting.

## Wire and generated bindings

`prost` generates private wire types from `core/proto`. These are converted to
domain values and do not cross public Rust or generated API boundaries. Control
frames have a 64 KiB Protobuf payload limit excluding the length prefix.

UniFFI metadata and Gobley 0.3.7 generate Kotlin declarations and implementations
over the same Rust-owned objects. The shared UI queries saved peers and invokes
pairing, reconnect and forget. The Android launcher supplies its private state
directory. APK packaging contains the shared core; unused sibling iroh-kmp
integration is not a build dependency.

## Growth rules

- Follow YAGNI; add protocols, traits, records and dependencies for current needs.
- Keep traits focused; do not create wrappers solely to preserve obsolete APIs.
- Test core contracts and lifecycle first, with real Iroh for transport behavior.
- Apps are UI/adapters and come after core correctness.
- Automatic retry/discovery, D-Bus hosting and device features are separate work.
