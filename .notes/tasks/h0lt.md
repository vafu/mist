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
- `core/src/media/`: `MediaProvider`/`MediaWatcher` host seam, a player registry
  that diffs local and per-peer state, `IrohMedia`.
- `mist/v1/media.proto`: `MediaSnapshot` and `MediaCommandRequest` control
  frames, validated on encode and decode.
- Snapshots are pushed on connection install and rebroadcast on local changes;
  remote players are cleared when a connection closes.
- Contract: `Media`, `MediaCommandHandler` (callback interface),
  `publish_local`, capabilities as a command list.

Linux (`mistd`):
- `apps/cli/src/media/mpris.rs` discovers players over MPRIS and forwards
  transport commands.
- `apps/cli/src/media/publish.rs` exports remote players as
  `org.mpris.MediaPlayer2.mist.<id>`, so `playerctl` and desktop widgets drive
  the remote device; exported `mist.*` names are excluded from discovery.
- D-Bus: `MediaPlayers` property and `MediaCommand(player_id, command)`.

Android (`:mist` service):
- `AndroidMedia` discovers sessions via `MediaSessionManager` using the
  notification-listener component, publishes them with `Media.publish_local`,
  and applies inbound commands through `MediaController`.
- `MistMediaSession` exposes remote players as a local `MediaSession` with a
  media notification: lock screen, notification card, Bluetooth, and hardware
  media buttons can control the remote device.
- `MistMediaButtonReceiver` routes `ACTION_MEDIA_BUTTON` to the service.
- `MediaCommandHandler` is registered through `createMistAppWithMedia`.

## Verified

- Linux MPRIS discovery and control: `testplayer` appears with title and artist;
  D-Bus `MediaCommand` moves it between Paused and Playing.
- Android emulator (`Pixel_3a_API_34`): active Google Podcasts session published
  and visible on Linux as a remote player.
- Linux → Android command routing reached `MistRuntimeService`.
- Remote Linux player reaches Android (`REMOTE:testplayer` in the runtime) and
  activates the `Mist` MediaSession with `Test Track` / `Test Artist`.
- Media notification posted on channel `mist_media`, `category=transport`, with
  Previous/Play/Next actions as real `PendingIntent`s.
- Media card renders in the notification shade showing the remote track
  (`Test Track` / `Test Artist`, Previous / Pause / Next).
- On the physical Pixel 10 Pro (build `382b155`): the `Mist` MediaSession is
  active with `state=PLAYING` and correct metadata, and the `mist_media`
  notification is posted with `template=android.app.Notification$MediaStyle`,
  `category=transport`, `ONGOING`, and 3 real PendingIntent actions.
- Core tests: remote visibility, command delivery to the owning provider, Added
  events, protocol round trip, invalid player rejection.

## Checks

`cargo test --workspace` (11 test binaries), strict Clippy, fmt, and
`./gradlew check :android:assembleDebug`.

## Known gaps

- A button press was not observed end to end. `adb shell` cannot drive the
  non-exported service, `cmd media_session dispatch` targets the media-button
  session (SoundCloud/telecom own it), and tapping shade coordinates while adb
  drives the device proved unreliable. Android also promotes the most recently
  active session, so the Mist card is not always the visible one. The dispatch
  path is implemented and each link is verified separately.
- Artwork is a URL only; no transfer.
- No per-device media enable/disable policy.
