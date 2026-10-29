# Mist working rules

## Trait-first Rust architecture

- Traits are mandatory contracts for all core behavioral components, including
  sessions, pairing, identity/device storage, and connection management.
- Define the trait contract before implementing a component. Consumers depend on
  the trait rather than a particular implementation; wire concrete implementations
  at composition boundaries.
- Keep runtime and connection behavior trait-backed and focused. Do not
  reintroduce the removed mixed local-state/single-connection `Session`, its
  getters, or a separate session layer without a concrete need.
- Keep traits focused on their domain responsibilities. Data records and error
  enums remain ordinary Rust values; behavioral interfaces belong in traits.
- Use `mod.rs` for contracts, domain types, and intentional re-exports. Put
  concrete behavioral implementations in sibling files, not module roots.
- Test core contracts and lifecycle behavior, using test implementations where
  useful and real Iroh integration where transport behavior matters.

## Shared core and priorities

- Follow YAGNI: add fields, types, protocols, traits, and extension points only
  for an established current need. Trait-first governs required behavior; it
  does not justify speculative services or one-to-one wrappers around every helper.
- Remove unused scaffolding rather than keeping it for hypothetical features.

- Rust owns domain behavior. There is one shared core API, not separate mobile
  and non-mobile APIs. Generated bindings expose the same core contracts.
- Apps are UI/adapters over the core. Core correctness and tests come first;
  app implementation is secondary and last priority.
- Linux and Android are both targets. Peers are symmetric: either side may
  initiate a connection, and multiple concurrent peer connections are required.
- Start device management with persistent local and trusted peer identities.
  Do not introduce snapshots, subscriptions, or other observation abstractions
  without an established requirement.

## Workflow

- Use `proj` for durable goals, subtasks, and design notes.
- Apply `rust-guide` from
  `/home/vfuchedzhy/proj/ai-docs/skills/rust-guide/SKILL.md`, reading the references
  relevant to the current phase. If unavailable, report it rather than inventing
  its rules.
