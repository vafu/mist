# mist-core Refactor Notes

## Final implementation status

The approved core/runtime/device-management plan is implemented. Runtime,
Connections, Pairing, Devices and DeviceStore define the behavioral boundaries.
FileStore and MemoryStore share the same persistence contract; Peer is reused
for pairing results and stored trust. The local identity and saved peers survive
restart, incoming acceptance has one owner, TLS exporter rank resolves reconnect
collisions, and forget is serialized against connection registration.

Core construction/operations use core-owned execution and one RuntimeError.
Shutdown and drop cleanup stop owned work, including abandoned construction.
Twenty-eight Rust tests pass along with strict Clippy, fmt/diff checks and
Rustdoc. Tests cover repeated pairing, persistent restart, symmetric reconnect,
concurrent connections, save/remove failures, stale trust reads, corruption,
writer exclusion, and cancellation/shutdown.

Linux uses persistent FileStore and device list/details/forget commands. Android
supplies private storage and retains the same Rust handle in a ViewModel across
configuration changes; the shared composable renders it and calls core APIs.
Binding generation, arm64 native compilation, Kotlin compilation, Android lint,
and debug APK assembly pass. Android host-test tasks currently have no sources;
no on-device runtime/UI test was claimed. See docs/architecture.md for the current
architecture and proj for completed task metadata.

The remaining sections retain the original review rationale/history. Historical
statements about missing behavior are superseded by this status and current docs.

## Earlier implementation update: contracts and ownership

Task f2ed99ad8f104bebb8b1eabf814457ba is implemented. Runtime and Connections
traits live in runtime/mod.rs; implementations live in runtime/service.rs and
runtime/connections.rs. Session is removed. Pairing receives local metadata and
contract dependencies directly. Runtime lookup is peer-addressed; raw endpoint
getters and global single-peer guards are gone. The peer map filters closed
connections and permits replacement. No additional runtime facade struct,
Session abstraction, or executor service was added.

All 14 tests pass, plus Clippy -D warnings, fmt/diff checks, and Rustdoc with
warnings denied. Tests include an injected connection implementation, runtime
trait-object transport, multi-peer pairing, and independent peer replacement.
The sections below preserve the review's pre-migration findings and decisions;
they are not claims that Session still exists. Execution/error unification,
central incoming dispatch/collision handling, and full shutdown guarantees
remain the next ordered tasks.

## Current Role

Shared native runtime: creates an Iroh endpoint, authorizes peers using
single-use invitations, persists local identity and remote trust, and manages
multiple peer connections with symmetric reconnect.
Produces both an rlib and cdylib. UniFFI exposes Rust-owned behavior through
generated bindings.

## Public Surface

`core/src/lib.rs` exposes:

| Exports | Current role |
| --- | --- |
| DeviceInfo, Peer, Platform | Device metadata and remote peer identity |
| DeviceStore, FileStore, MemoryStore, DeviceError | Identity and trusted-peer storage |
| Devices | Saved-device operations |
| Pairing, PairingOffer, PairingError | Pairing contract and values |
| MistBuilder | Endpoint/runtime construction configuration |
| Runtime, MistRuntime | Shared runtime contract and implementation |
| RuntimeError | Shared runtime failures |
| create_mist_runtime | Async construction entrypoint |
| MIST_ALPN | Iroh application protocol identifier |

Core behavior uses focused traits; data records and error enums remain values.

## Internal Structure

- `device/mod.rs`: metadata, Peer, storage/device contracts and errors.
- `device/file.rs`, `memory.rs`, `service.rs`: persistence and device operations.
- `runtime/mod.rs`: runtime/connection contracts, errors and reexports.
- `runtime/service.rs`, `connections.rs`: runtime and incoming/peer lifecycle.
- `runtime/builder.rs`: endpoint configuration and binding.
- `runtime/executor.rs`: shared Tokio executor for construction.
- `pairing/mod.rs`: trait and domain values/errors.
- `pairing/service.rs`: Iroh pairing implementation.
- `protocol/mod.rs`: private generated messages, framing and conversions.
- `proto/mist/v1/`, `build.rs`: wire schemas and Prost generation.
- Module `test.rs` files and `tests/session.rs`: current tests.

## Behavior Summary

Construction loads local identity before binding on core-owned execution.
Pairing authorizes a token before metadata exchange and stores trust before
reporting local success. Reconnect requires saved trust; forget removes it and
closes the peer. The endpoint accept loop is core-owned, not exposed to apps.

## User Notes

Preserved intent from the discussion:

- "traits for everything"; Session specifically needs a trait.
- "we don't distinct 'mobile' and 'non-mobile' api"; apps are adapters.
- "any side can initiate connection. multiple connections are supported."
- "both linux and android, with core correctness as a first main goal."
- "why do we need the snapshot at all?" Identity storage is the immediate need.
- "follow YAGNI pattern ... only add things if we need them ... keep platform"
- "yes, simplify to android/linux/other. next" — implemented for Platform.
- "impl should not be in mod. needs a trait" — module roots define contracts;
  concrete behavior belongs in sibling implementation files.
- User asks what Session represents, why it exposes an executor, and who uses
  it. Resolve existing responsibilities before accepting a new Session model.
- User confirmed that Connection is per remote peer while the local instance
  must support a network of devices. Do not mechanically preserve the current
  mixed local-state/single-connection Session behind a trait.
- User agreed to remove PairedPeer in favor of one reusable Peer record, with
  as few structs as practical. Renamed and moved the record to device.rs;
  pairing now returns Peer. No separate storage/trusted-device DTO introduced.

## Findings

### First reading stop: lib.rs and device.rs

- lib.rs describes single-peer runtime ownership accurately for the bootstrap,
  but that invariant conflicts with the agreed multi-peer direction.
- DeviceInfo is metadata, not an endpoint identity or trust record. Keeping that
  distinction avoids giving a display name authorization meaning.
- DeviceClass and unused capability schema scaffolding were already removed.
- Platform now contains Android, Linux, Other following explicit user approval.
  Removed named unsupported platforms from Rust and Protobuf conversions;
  reserved deleted Protobuf enum numbers/names. Unknown wire values still map
  to Other.
- Naming/docs that frame construction or API behavior as mobile-specific need
  review against the one-core-API rule.

### Second reading stop: runtime/mod.rs

MistRuntime stores Arc<Session> and Arc<dyn Pairing>. Session owns Endpoint,
DeviceInfo, OnceLock<Connection>, and a Tokio Handle. Runtime constructs the two
implementations and delegates most operations to Session. Session therefore
currently behaves as process/endpoint state rather than a per-peer session.

Factual issues: one permanent connection slot prevents multiple peers and
replacement after closure; is_connected checks installation rather than live
connection status; Session has no trait and pairing depends on its concrete
type. Raw connect/accept install connections without pairing authorization.

Superseded proposal: introducing a per-peer Session contract solely to replace
the existing struct is not justified. User confirmed the separation below.

Accepted ownership direction: one local instance owns one endpoint and local
identity/metadata; connection management handles connections to multiple remote
peers symmetrically. Saved trust is separate from active connections. Define a
focused connection-management trait for this required behavior. A separate
Session abstraction needs a demonstrated responsibility beyond Iroh Connection.

#### Existing consumers and executor rationale

- Only production owners of Arc<Session> are MistRuntime and IrohPairing.
  MistRuntime::from_endpoint constructs it and shares the same allocation.
- Runtime delegates endpoint/connection/connect/accept and shutdown to Session;
  endpoint_id/is_connected also directly inspect its fields.
- Pairing reads local metadata for hello, endpoint address for invitations,
  connection occupancy for AlreadyPaired checks, opens outgoing candidates,
  accepts incoming candidates, and installs authorized connections.
- Session captures Handle::current() at construction. Pairing::accept and
  wait_for_peer use executor() to spawn their entire operations; shutdown does
  the same. The accessor is pub(crate), not an exported application API.
- create_mist_runtime constructs on the shared Tokio runtime; builder.bind
  captures the caller's active Tokio runtime instead. Scheduling operations onto
  the captured runtime supports calls polled without a host Tokio reactor.
- This is execution plumbing leaking through the shared state object, not an
  inherent peer-session responsibility. It does not yet justify an Executor
  trait; determine minimal runtime ownership and hide scheduling in implementation.
- Session currently means local running-instance state plus a single connection,
  not a persistent device record or a well-defined per-peer session. Earlier
  per-peer Session suggestion is a redesign proposal, not current behavior.

Evidence: source reads and focused search after lspctl references failed with
server_selection_failed / No route matches the target file. No semantic query
result is claimed.

## Refactor Plan

### Consolidated implementation order

The authoritative checklist/status is in proj. This is the dependency map and
code direction agreed during review, not a second status checklist.

| Order | proj task | Focus and evidence |
| --- | --- | --- |
| 1 | f2ed99ad8f104bebb8b1eabf814457ba | Define runtime/connection contracts in module roots; move concrete runtime implementation to sibling files and migrate consumers away from the mixed Session. Preserve current working regression tests; document lifecycle invariants. |
| 2 | 0895b72f715a4948bb348bc996e6c4d1 | Unify builder/factory execution and runtime errors; private core-owned scheduling. Test no-host-Tokio operations and independence from caller-runtime lifetime. |
| 3 | 4f1641ed363c4fa5abc783c705ebecf5 | Symmetric multi-peer connections, single incoming owner, pairing candidates, shared authorized registration, same-peer collision and stale-close correctness. Real-Iroh multi-instance tests. |
| 4 | 5df5cb0bff0e48acb7c3367488ee2c4c | Complete idempotent shutdown/task lifecycle and cross-instance isolation; final workspace checks. |

Files primarily affected: runtime/mod.rs, runtime/builder.rs,
runtime/executor.rs, new focused sibling implementation files, pairing/mod.rs,
pairing/service.rs, and core tests. New module names and signatures follow the
smallest coherent ownership split. Keep public contracts discoverable through
lib.rs; generated bindings expose the same domain behavior. Existing callers
must compile as APIs migrate; app feature work is last priority.

Each task carries behavior checks rather than postponing all tests to a final
testing phase. All four tasks gate completion of the core architecture milestone,
even though the reused multi-peer task remains under the device-management goal.
Contract migration can be staged, but must not retain a speculative Session trait
just to make an intermediate architecture look finished.

Afterward: finalize identity/storage contracts, persist local identity, store
Peer records, integrate durable pairing/removal, then authorize trusted reconnect
(a6f27bc1e62b4157afd620a288cda983). Reconnect was split from the reused multi-peer
task to make the storage dependency explicit. Automatic retry/discovery and app
features are not requirements of the core cleanup.

### Third reading stop: pairing's actual connection dependencies

Outgoing pairing: parse invitation -> connect_candidate(address) -> token
authentication -> exchange local/remote metadata -> install(connection).
Incoming pairing: initiate creates invitation from local endpoint address;
wait_for_peer directly accepts candidates, authenticates token, exchanges metadata,
and installs the successful connection. Token state remains pairing-owned.

Required behaviors evidenced by these paths: open an outgoing connection and
register an authorized connection. An incoming path must supply candidates to
pairing. Local metadata/address are inputs; executor access and global connection
occupancy checks are not meaningful connection-management contract methods.

Accepted by user: endpoint acceptance has one core owner; pairing
handles a supplied incoming candidate rather than competing for endpoint.accept.
Keep public wait_for_peer meaning as waiting for the pairing result, not owning
all endpoint acceptance. Do not add a routing framework/event bus merely to
implement this ownership. Trait signatures remain to be defined.

### Fourth reading stop: invitation lifecycle and pairing completion

Current pending state is one Option<32-byte token>, carried in a watch channel.
initiate replaces the previous token; cancel clears it; invalid tokens do not
consume it; a valid token is atomically consumed before metadata exchange and
registration. No time expiry or persistence exists for invitations. One pending
invitation is separate from the number of active peer connections.

PairingAuthenticated confirms token authorization, not completed metadata exchange,
registered connection, or durable trust. Failure after that acknowledgement can
leave the invitation consumed with pairing incomplete. Existing tests verify
invalid-token nonconsumption, cancellation wakeup, and success; valid-token races
and failure after consumption are not covered.

Accepted by user: retain one pending single-use invitation,
replacement/cancellation invalidation, and consumption on valid authorization.
Do not restore consumed tokens automatically. Re-pairing after a failed authorized
handshake uses a new invitation. Define public pairing success as completed
handshake and registration, plus successful local trust persistence when added.
No claim of atomic distributed trust storage; persistence failure handling remains
to design. Avoid adding invitations database, expirations, and multiple offers
without a concrete requirement.

Single-peer guards in initiate, accept, wait_for_peer and AlreadyPaired's current
global meaning conflict with adding a new peer while other peers are connected.
Removing those guards alone is insufficient while Session still has OnceLock.

Accepted constraints: trait-first behavior and YAGNI, shared API, symmetric
multi-peer core, tests first. Resolve the existing Session by separating local
instance state from per-peer connections, not by adding a trait for its current
bag of getters. Concrete contract signatures remain undecided.

Accepted layout rule: move concrete runtime/session implementation out of
runtime/mod.rs; keep trait contracts and intentional public surface discoverable
there. Define responsibilities first, rather than copying the existing oversized
Session API into a trait or inventing an executor service.

Make only explicitly approved changes during this interactive review.

## Tests And Verification

After approved Platform simplification: 11 Rust tests passed; workspace Clippy
with -D warnings, fmt check, and diff check passed.
After Peer rename: all 11 tests and workspace Clippy passed; generated record
metadata compiles. Android binding generation/build not run.
Existing tests are evidence for bootstrap behavior, not proof of future
multi-peer/storage contracts.

## Review discussions (historical)

### Runtime construction and executor ownership

Current call paths:

- MistBuilder::bind binds directly on the caller's Tokio runtime and captures
  Handle::current via Session::new. Its endpoint tasks therefore depend on the
  host runtime remaining alive.
- create_mist_runtime schedules that same builder on a process-wide lazily
  initialized Tokio runtime from executor.rs. It accepts DeviceInfo only, so
  cannot express secret_key, local_only, transport, or lookup configuration.
- Factory maps typed MistError::Bind into RuntimeError::Bind reason text.
- Pairing accept/wait and runtime shutdown spawn on the captured handle, while
  raw runtime connect/accept directly execute in the polling caller's context.
- Existing no-host-Tokio test covers factory creation/endpoint ID/shutdown only,
  not builder construction, pairing, or direct connect/accept.

Accepted by user: one construction implementation
using core-owned execution, independent of host polling runtime. Builder bind
owns this path; exported default constructor delegates to it without separate
executor/error policy. Core execution remains private implementation machinery,
not a domain getter/service. Keep current builder configuration that has actual
uses; align bindings as requirements become concrete rather than inventing a
parallel config model or exposing raw Tokio types.

Process-wide runtime ownership is an existing implementation option, not a
requirement for one Tokio runtime per Mist instance. Shutdown closes Mist-owned
resources, not an executor shared with other instances. Preserve custom transport
runtime requirements when investigating implementation.

Verification needed for accepted future refactor: builder and default constructor
work when polled without Tokio; actual pairing/connection lifecycle works in that
context; a runtime built while a temporary host Tokio runtime exists can continue
after that host runtime is dropped; shutting down one Mist instance leaves other
instances usable. Apply the agreed single domain error contract.

Evidence: direct source reads after named references query failed with
server_selection_failed / No route matches the target file. This review step
changes notes only, not executor behavior.

### Next reading stop: shutdown and cancellation

Current shutdown spawns Session::close on captured Tokio handle, closes the
single connection and endpoint, and ignores join failure. Pairing operations
spawn their own tasks; dropping caller futures does not abort these tasks.
cancel() clears the invitation, but shutdown does not explicitly invalidate it
or join pairing tasks. Endpoint closure can wake pending acceptance; after a
candidate token is accepted, inline metadata exchange has different cancellation
behavior. No general task-lifecycle contract exists yet.

Accepted lifecycle contract for the runtime refactor: shutdown is
idempotent, rejects new work once closing starts, invalidates invitations,
stops/joins instance-owned work, and closes all peer connections plus endpoint.
Return errors from failed shutdown execution rather than silently swallowing
them. Leave the shared executor and other instances alive. Define this with
deterministic lifecycle tests; no new scheduler/event framework is implied.

### Next reading stop: test strategy

Keep existing real-Iroh pairing and transport tests as regression coverage while
replacing single-peer assumptions. Twelve tests currently pass, including the
new framing boundary test. Accepted lifecycle/construction/connection ownership
changes are still design decisions, not completed behavior.

Proposed implementation verification order: trait contract and minimal test
implementation where useful; real-Iroh test with three instances connected in
both directions; concurrent attempts to the same peer with agreed collision
semantics; closure/replacement and stale-close correctness; pending-pairing
shutdown and repeat shutdown; construction and operations without caller Tokio;
isolation between instances. Persistent identity/trust tests follow when storage
is implemented. Use barriers/channels to synchronize; timeouts bound hangs, not
protocol timing. Do not add tests that only assert getters or struct layout.

### Next discussion: error boundaries

Current public errors: MistError (typed Iroh errors for builder/direct transport),
RuntimeError (factory-only Bind/Executor reason strings), PairingError (operation
categories and messages). Private ProtocolError covers framing/hello validation.
MistError::Bind and RuntimeError::Bind represent the same endpoint-bind failure
through different construction paths. Representation differs because direct
Rust errors carry Iroh sources while the exported factory uses UniFFI-compatible
values; this does not justify different domain semantics.

Accepted by user: one public runtime error contract across both
construction paths; keep operation-specific pairing errors and private protocol
detail rather than collapsing every error into one enum/string. Remove global
AlreadyPaired/ConnectedToDifferentPeer restrictions as part of multi-peer work,
not in isolation while the single-connection implementation still depends on
them. Check UniFFI representation against existing dependencies before changing
error implementation. No error refactor performed in this review step.

### Next reading stop: protocol surface

Private Protobuf values are converted to DeviceInfo; separate wire/domain values
have a real boundary purpose, unlike duplicate pairing/storage records. Keep
wire types out of the public API. Current messages implement token auth/rejection
and metadata hello only; unused capabilities have already been removed.

User approved moving implementation to a sibling file and a consistent 64KiB
Protobuf payload limit, excluding length prefix. Implemented in protocol/codec.rs:
writer checks encoded_len before encoding; reader checks payload length before
allocating. mod.rs retains constants, errors, wire types and reexports. No codec
service struct or speculative trait was added for these private helpers.

Added real-Iroh regression test verifying a 64KiB frame travels successfully,
a 64KiB+1 frame is rejected by the writer without emitting bytes, and a raw
oversized length prefix is rejected by the reader without needing a payload.
All 12 workspace tests, Clippy with -D warnings, fmt and diff checks passed.

- Define storage commit/failure handling when persistence is introduced;
  invitation consumption/retry semantics are agreed. Trait signatures remain
  to be specified.

## Reading Progress

Interactive read-through and the resulting approved implementation plan are
complete. Final verification is recorded above and in the proj goal notes.
