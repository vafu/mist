---
tags:
- task/3cd48924bffc4092823c8baad49aa411
- note/evidence
- kind/evidence
title: Notification images
---
# Notification images

## Implemented

Contract and wire:
- `NotificationImage { mime_type, path }` on `Notification.app_icon` and
  `Notification.image`, both optional.
- `mist/v1/notification.proto` gains `NotificationImage` metadata plus tags 8/9
  on `Notification`. Image bytes follow the control frame on the same stream,
  in order: app icon, then content image. No new ALPN.
- Limits: 512 KiB icon, 8 MiB image. Sizes are validated in `push_notification`
  before scheduling, so oversize artwork fails at the call site rather than
  inside the outbox task.
- Images stream through the existing blob primitive, so memory stays bounded and
  hashes are verified.

Linux:
- Outbound: reads `image-path` / `image_path` / `app_icon` hints and the
  notification's own icon from the freedesktop `Notify` call.
- Inbound: attaches received artwork as an `image-path` hint.
- CLI: `mist notify <title> [body] --image <path>`.

Android:
- Outbound: `smallIconImage()` renders the notification's small icon to PNG and
  `largeImage()` extracts `EXTRA_PICTURE` / `EXTRA_LARGE_ICON`.
- Inbound: app icon becomes `setLargeIcon`; content image becomes `BigPictureStyle`.
- `Bundle.getParcelable(String, Class)` requires API 33, so the extraction uses
  the deprecated single-argument form, which works from minSdk 26.

## Verified

- Core tests: `notification_images_cross_the_wire` (5 KB icon plus 40 KB image,
  bytes identical on receive), `notification_image_path_is_populated_on_receive`,
  and `oversized_notification_image_is_rejected`.
- On the Pixel: `mist notify "Img4" ... --image /tmp/mist-blob-test.png` produced
  `notif image path=.../mist/clipboard/867347ed….blob mime=image/png exists=true`
  and `notif image decoded=true`.
- Pulled the received blob off the device: 69,716 bytes, valid 640x480 PNG,
  byte-identical to the source.
- The Mist notification group renders in the shade with the bridge app icon.

## Caveat

`dumpsys notification` reports `android.picture=null` for `BigPictureStyle`
notifications regardless of whether the bitmap is attached, so that field is not
a reliable check. The decode log and the identical on-device blob are the
evidence that the image transferred and rendered.

Google's Smartspace/BlobDB echoed the Mist notification back into Android's own
notification pipeline (`BlobDB` insert), which is unrelated noise in logcat.
