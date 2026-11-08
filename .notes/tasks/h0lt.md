---
tags:
- task/cec90b7838a04ba69ccc61f7200fd31f
- note/evidence
- kind/evidence
title: Media verification
---
# Media player component (MPRIS + Android MediaSession)

## Implemented

Core (`mist-core`):
- `core/src/media/` with `MediaProvider`/`MediaWatcher` host seam, a player
  registry that diffs local and per-peer state, and `IrohMedia`.
- Wire protocol: `MediaSnapshot` and `MediaCommandRequest` control frames
  (`mist/v1/media.proto`), validated on encode and decode.
- Snapshots are pushed to a peer on connection install and rebroadcast on local
  changes; remote players are cleared when the connection closes.
- Contract: `Media`, `MediaCommandHandler` (callback interface for generated
  bindings), `publish_local`, capabilities as a command list.

Linux (`mistd`):
- `apps/cli/src/media/mpris.rs` discovers players over MPRIS, reads metadata and
  capabilities, and forwards transport commands via `MediaCommand`.
- `apps/cli/src/media/publish.rs` exports remote players as
  `org.mpris.MediaPlayer2.mist.<id>` so desktop widgets and `playerctl` control
  the remote device.
- D-Bus surface: `MediaPlayers` property, `MediaCommand(player_id, command)`.
- Our own exported `mist.*` names are excluded from discovery (self-loop fix).

Android (`:mist` service):
- `AndroidMedia` uses `MediaSessionManager` with the notification-listener
  component, publishes active sessions through `Media.publish_local`, and
  applies commands through `MediaController`.
- `MediaCommandHandler` callback registered through
  `createMistAppWithMedia`; no Binder transport for native handles.

## Verified

- Linux MPRIS discovery: synthetic player appears as `testplayer` with title and
  artist; `MediaCommand pause/play` moves `playerctl` between Paused and Playing.
- Android emulator (`Pixel_3a_API_34`): active Google Podcasts session published
  and visible on Linux as a remote player from endpoint `271f215a…`.
- Command routing Linux → Android: `MediaCommand` reached
  `MistRuntimeService` as `media command PLAY/PAUSE for <token>`.
- Command routing Linux local: `testplayer` responded to D-Bus commands.
- Core integration tests: remote state visibility, command delivery to the
  owning provider, and `Added` events for local players.
- Protocol tests: snapshot/command round trip and invalid player rejection.

## Checks

`cargo test --workspace` (11 test binaries, all passing), strict Clippy, fmt,
and `./gradlew check :android:assembleDebug`.

## Known gaps

- Android receives remote players in the runtime but has no UI or local
  `MediaSession` publishing them as controllable players; there is no
  `MistPlayerActivity`/session yet, so a remote player is not controllable from
  the Android media notification.
- Artwork is a URL only; no transfer.
- No per-device media enable/disable policy.
