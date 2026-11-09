---
tags:
- task/3cd48924bffc4092823c8baad49aa411
- note/evidence
- kind/evidence
title: Notification actions
---
# Notification actions

## Model

Actions are metadata plus a return path, so they are two frames:

- `Notification.actions` — `NotificationAction { id, label }`, max 8, id/label
  capped at 128 chars. Travels with the notification.
- `NotificationActionActivated { notification_id, action_id }` — a new control
  frame (tag 11) sent back to the originating device.

Ownership is by design: only the device that produced a notification can perform
its action, so activation travels from the *receiving* device back to the
*sending* device and is delivered there through `subscribe_activations`.

Validation: the receiver keeps the action ids of notifications it sent (bounded
at 256 entries, cleared wholesale when full) and drops activations for unknown
notifications or unknown action ids. This prevents a peer from invoking arbitrary
actions and keeps the store from growing without bound.

## Platform wiring

Linux:
- Outbound `Notify` sends the flat `[id, label, ...]` action list. A synthetic
  `default` action is added when none exist (body click) and filtered out when
  reading, since it is not a button.
- Inbound maps the desktop notification id returned by `Notify` to
  (endpoint, notification id, action ids), watches the `ActionInvoked` signal on
  a monitored connection, and calls `NotificationChannel::activate`.
- The mapping is bounded at 256 entries.

Android:
- Capture reads `Notification.actions` and assigns sequential ids (`a0`, `a1`,
  …) because `Notification.Action` exposes no identifier.
- Receive renders each action as a notification button whose `PendingIntent`
  carries endpoint, notification id, and action id back to the service.
- Activations from peers are pushed to the Binder client through
  `IMistCallback.onNotificationAction`, exposed as
  `AndroidMistController.subscribeNotificationActions`.

## Verified

Core, with two paired runtimes:
- `notification_actions_round_trip_to_the_owner` — actions survive the wire with
  ids and labels intact, and activating `dismiss` on the receiver arrives at the
  sender as `ActivatedNotificationAction` with the right endpoint.
- `unknown_notification_actions_are_ignored` — an activation for a notification
  the sender never produced is dropped and never surfaced.

Checks: `cargo test --workspace` (11 binaries), strict Clippy, fmt, and
`./gradlew check :android:assembleDebug`.

## Not yet verified on device

The Pixel dropped off ADB before I could tap a rendered action and observe it
fire on Linux (and vice versa). The platform code paths compile and lint, but the
end-to-end tap was not exercised on hardware.

Android note: `Notification.Action` has no public id, so ids are positional.
Two notifications with identical action sets still work because activation is
keyed by (endpoint, notification id, action id).
