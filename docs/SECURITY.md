# Security

AirBridge moves files between your own devices on a local network. This page
says what it protects against, and how.

## Who we defend against

- **Someone else on the same Wi‑Fi** (a café, an office, a shared flat): they
  can send any packet to your devices, pretend to be any device, and read
  anything sent over plain HTTP.
- **Whatever a sender sends**: hostile file names, sizes, previews and texts.
- **A web page open in your browser**, trying to reach the app's local API.
- **Other user accounts on the same computer**, and **other apps on the phone**.

## What protects you

**Who is who.** Every device has its own certificate. A device is *verified*
only when it proved its identity with that certificate during the TLS
handshake. Only verified devices you trust ("my devices") get the AirDrop-like
conveniences: accepted without asking, opened right away, text copied to the
clipboard. Anyone can *claim* a device's id (fingerprints are broadcast), so a
claim without the certificate is refused for your devices and for any device
that proved itself before. A discovery packet alone never adds or changes a
device; it is confirmed over TLS first. A known device is never moved from
HTTPS to plain HTTP, your own devices are only reached over HTTPS, and a PIN is
only sent over HTTPS.

**Pairing by QR code.** The code is the only way a device becomes trusted
without a tap on both ends, so it is treated as a secret you show in person. It
carries the certificate fingerprint to expect, a one-time key (5 minutes on the
computer, the sharing session on a phone) and, from a phone, the Wi‑Fi Direct
details. The scanner connects with that fingerprint pinned, so the key is only
ever sent to the device that showed the code; a mismatch aborts before it is
sent. The key is only accepted with a client certificate, and that certificate
is what the host then trusts. A scanned code is untrusted input (a QR sticker
can say anything): it must be well formed, must name a literal IPv4 address and
a `DIRECT-` network, and nothing is trusted until you confirm the device by
name on the scanning phone. Paired devices remember each other's private IPv4 addresses,
including Tailscale addresses, and the private address they connected from. Only addresses in
`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16` and the shared CGNAT range
`100.64.0.0/10` are ever remembered or probed for reconnection. Every reconnect probe pins
the paired certificate, so another device at an old address cannot take its place. Removing
trust also removes the remembered addresses.

**What arrives.** A sender cannot write more than the size you accepted (it is
checked while data streams in), cannot write outside the download folder (no
`..`, no symlinks, no overwriting, no hidden or too-long names, no invisible
right-to-left tricks), and its preview pictures are re-encoded before anything
else parses them. Only photos, videos, music, PDFs and plain text ever open by
themselves; anything runnable (scripts, `.desktop` files, AppImages, HTML,
SVG, APKs) only shows in its folder. Links open by themselves only from your
own verified devices, and only `http`/`https` links.

**Floods.** Connections are capped per address and in total; slow or idle
connections time out; requests that reach you (prompts and messages) are
rate-limited per address; wrong PINs are limited per address *and* in total,
so using many addresses does not help; PINs need at least 6 characters; the
device list has a size limit; Quick Share payloads and file counts are bounded.

**On the computer.** The window's API listens only on 127.0.0.1, checks the
Host header (no DNS rebinding) and needs a secret token in a strict cookie.
The token never appears in a URL: the window opens with a one-time code that
expires after a minute. Pages run only their own scripts (Content Security
Policy) and cannot be framed. Settings and keys are written privately (0600,
no symlinks, in folders that must be yours). Received files cannot be routed
into hidden or system folders.

**The phone browser page** (for phones without the app) talks plain HTTP on
your Wi‑Fi, so its pairing key could be overheard. It is off until you turn it
on, and everything it sends asks you first, like any other sender.

**On Android.** The share sheet only sends content that another app hands
over (`content://`), never AirBridge's own files, and sends without a tap only
to your own verified devices. Devices at the phone's own address are refused
(another app could pose as one). App data is excluded from backups and device
transfers, received text is marked sensitive on the clipboard, and malformed
requests cannot crash the app.

## Updates

The Linux AppImage and the Windows exe update themselves only from stable `vMAJOR.MINOR.PATCH`
releases in the public [MDHaarith/Airbridge](https://github.com/MDHaarith/Airbridge) repository, over
HTTPS (system CA store, HTTPS-only redirects). CI signs each AppImage with an
Ed25519 key kept in the `UPDATE_SIGNING_KEY` secret. The signature covers the
file and its tag, and the public key is built into the app. A download with a
bad signature, an older or equal version, or one that is not an ELF file
is rejected before anything is written. The swap is atomic (temp file, fsync,
rename). Only the token-protected local UI can start an update.

## Known limits

- The official LocalSend apps do not present client certificates. They work,
  but always ask, and cannot become "my devices".
- Quick Share senders have no lasting identity (their name and address are
  their only id), so they always ask too. Compare the PIN.
- The Android app updates from the same releases. It has no signature of its own
  on top: Android itself refuses an APK that is not signed with the installed
  app's release key.

## Reporting a problem

Please open an issue, or contact the maintainer privately for anything that
should not be public yet.
