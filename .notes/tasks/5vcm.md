---
tags:
- task/cec90b7838a04ba69ccc61f7200fd31f
- note/decision
- kind/decision
title: Media design
---
# Media player component (MPRIS + Android MediaSession)

## Goal

A playing track on either device is visible and controllable from the other.
Linux uses MPRIS; Android uses `MediaSessionManager` + `MediaController`.
The second half of a control round trip is handled by the platform, so the
shared runtime only transports state and commands.

## Layering

- `mist-app-contract`: `MediaPlayer`, `MediaState`, `MediaCommand`,
  `MediaPlayerEvent`, `MediaError`. App-visible values only.
- `mist-core`: one `Media` feature that owns player discovery state,
  selection routing, outbound command scheduling, and inbound command
  application through a narrow trait.
- Platform hosts: implement the narrow trait.
  - Linux (`mistd`): MPRIS via D-Bus (zbus), both directions.
  - Android (`:mist` service): `MediaSessionManager` for discovery/state and
    `MediaController` for transport commands.

## Core contract

```rust
#[uniffi::export]
#[async_trait]
pub trait Media: Send + Sync + Debug {
    /// Local and remote players known to this runtime.
    fn players(&self) -> Vec<MediaPlayer>;
    /// Reactive stream of player/state changes.
    fn subscribe(&self) -> Arc<dyn MediaEventStream>;
    /// Sends a transport command to a remote player.
    async fn command(&self, player_id: String, command: MediaCommand) -> Result<(), MediaError>;
    /// Publishes this device's local player state to peers.
    async fn publish_local(&self, state: MediaState) -> Result<(), MediaError>;
}
```

`MediaPlayer` carries an `origin` (`Local`/`Remote`), endpoint id for remote
players, stable player id, metadata (title, artist, album, art URL), playback
status, and capabilities.

## Platform trait (host seam)

```rust
pub trait MediaProvider: Send + Sync + Debug {
    fn watch(&self) -> Result<Box<dyn MediaWatcher>, MediaProviderError>;
    fn command(&self, player_id: &str, command: MediaCommand) -> Result<(), MediaProviderError>;
    fn publish(&self, state: &MediaState) -> Result<(), MediaProviderError>;
}
```

Linux: MPRIS watcher over `NameOwnerChanged` + `PropertiesChanged`; publish by
exporting `org.mpris.MediaPlayer2.mist`.
Android: `MediaSessionManager.OnActiveSessionsChangedListener`; publish by
keeping a `MediaSession` owned by `MistRuntimeService`.

## Wire protocol

Add `MediaStateChanged` and `MediaCommand` control frames (no new ALPN).
Commands carry the source endpoint and target local player id; the receiving
host applies them through its provider. Local state changes are broadcast to
selected/connected peers.

## Non-goals

- No artwork transfer in v1 (URL only).
- No playlist or queue browsing.
- No per-track history.
