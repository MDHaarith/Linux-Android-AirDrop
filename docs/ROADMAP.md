# Roadmap: the "AirDrop like Mac and iPhone" milestone

Goal: sharing between a KDE laptop and an Android phone should feel like
AirDrop between a Mac and an iPhone: the devices just see each other, from
the share sheet on the phone and from anywhere on the desktop.

## 1. Quick Share interoperability (Linux, Rust)

A stock Android phone shares to the laptop from its built-in Quick Share
sheet, with no app installed.

- [x] Vendored Nearby protobufs (Google, Apache-2.0) + generated prost code
- [x] UKEY2 handshake (P-256), SecureMessage (AES-256-CBC + HMAC-SHA256), PIN
- [x] mDNS advertisement `_FC9F5ED42C8A._tcp` with endpoint info
- [x] Inbound: files, text, URLs, Wi-Fi credentials (shown, not applied)
- [x] Transfers show up in the same UI, notifications and history as LocalSend ones
- [x] Outbound: discover phones in receive mode, send files and text
- [x] BLE "someone is sharing" trigger via BlueZ so phones start advertising
      (contract-tested against a fake BlueZ; not yet tried on real hardware)
- [x] Settings: Quick Share on/off, visibility, device name
- [x] Tests: crypto known answers, inbound/outbound loopback through the
      real code paths

## 2. Drop zone on the screen edge (KDE Plasma, Wayland and X11)

Drag anything (an image on a web page, files from Dolphin, selected text)
to the edge of the screen: a panel slides out with nearby devices, drop on
one and it is sent.

- [x] `airbridge edge`: GTK3 window, wlr-layer-shell on Plasma Wayland,
      dock window on X11; click-through except while dragging
- [x] Accepts file URIs, image data, URLs (downloaded by the service), text
- [x] Animated panel: slide-in, device bubbles, hover glow, send progress
      ring, success burst
- [x] Started by the service; settings for edge and enable/disable
- [x] Tested under Xvfb with a synthetic drag (X11) and under headless sway
      (layer-shell rendering); offscreen render snapshots in CI

## 3. The same on the phone (Android)

- [ ] Direct Share: nearby devices appear as targets in the Android share
      sheet; picking one sends immediately
- [ ] Edge handle overlay with a slide-out panel: devices, send clipboard,
      send latest screenshot/photo
- [ ] Image previews in "received" notifications

## Environment notes (for resuming in a fresh container)

- Android SDK: `/opt/android-sdk` (platforms;android-35, build-tools;35.0.0),
  `ANDROID_HOME=/opt/android-sdk`.
- Maven Central rate limits: use the Gradle init script that adds the
  `maven-central.storage-download.googleapis.com` mirror
  (`gradle --init-script mirror.init.gradle`).
- Linux build deps: `libgtk-3-dev libwebkit2gtk-4.1-dev libgtk-layer-shell-dev
  xvfb xdotool picom hsetroot` (+ `sway grim` for the Wayland check).
