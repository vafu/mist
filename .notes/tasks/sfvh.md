---
tags:
- task/3cd48924bffc4092823c8baad49aa411
- note/progress
- kind/fleeting
- task/cec90b7838a04ba69ccc61f7200fd31f
title: Android UI rewrite
---
# Android UI rewrite

## Done

- `MistApp.kt` rebuilt as a Material 3 screen: `Scaffold` + `TopAppBar`,
  lazy device cards, permission card, empty/loading/error states, snackbar
  error reporting, QR pairing `ModalBottomSheet`.
- New `ui/` package: sand light/dark `MistTheme`, vector `MistIcons`, and
  `AppMark`, which renders the launcher art (towers, cable, deck) instead of
  the earlier hand-drawn bridge that read as a croissant.
- Device card: switch now connects/disconnects; selection moved to a
  "Set destination" action. Core semantics unchanged.
- Ongoing service notification: header shows Connected / Connecting… /
  Not connected, body enumerates connected device names joined by ` · `,
  tapping opens `MainActivity`. Uses a monochrome `ic_stat_mist` status icon
  for both the service and forwarded notifications.

## Notes

- Pairing is QR-only; the debug paste field and endpoint ID display from the
  old UI were dropped.
- `rememberModalBottomSheetState` is deprecated in material3 1.12.0-alpha03.