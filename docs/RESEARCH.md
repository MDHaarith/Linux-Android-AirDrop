# Research: open-source AirDrop, and AirDrop-style sharing between Linux and Android

This is the research behind AirBridge: what exists, what each approach needs, and why
AirBridge is built on the LocalSend protocol.

## 1. How Apple AirDrop works (and why Android can't join)

AirDrop has three layers ([OpenDrop][opendrop], [OWL][owl]):

1. **Bluetooth LE advertisements.** Senders broadcast short hashes of their contact identifiers to
   wake nearby receivers.
2. **AWDL (Apple Wireless Direct Link).** This is a proprietary peer-to-peer Wi-Fi link. Devices
   hop between channels in sync and talk directly, even without a shared access point.
3. **HTTPS over AWDL.** Devices find each other with DNS-SD (`_airdrop._tcp`), then the sender
   calls `/Discover`, `/Ask` and `/Upload` with a gzipped cpio archive. "Contacts only" mode
   needs Apple-signed certificates and validation records.

Open implementations exist:

| Project | What it is | Requirements and limits |
|---|---|---|
| [OpenDrop][opendrop] (seemoo-lab, Python, GPLv3) | Re-implements the AirDrop protocol. Talks to real iPhones and Macs. | Needs AWDL: macOS, or Linux with OWL. Contacts-only mode needs credentials extracted from a Mac keychain. Not all AirDrop features, and may break with newer AirDrop versions. |
| [OWL][owl] (seemoo-lab, C) | User-space AWDL for Linux. | A Wi-Fi card with **monitor mode and frame injection**. Not something a stock Android phone can run. |
| Newer experiments (news reports, Oct 2026) | Reported AWDL-in-userspace tools for specific chipsets (e.g. MediaTek MT7925) | Experimental and driver-dependent. Reported speeds are around 1 MB/s. |

**Conclusion.** Real AirDrop needs AWDL. Android phones don't have AWDL and can't run OWL
(no monitor mode or frame injection without root and special hardware). So "AirDrop between
Linux and Android" has to mean an AirDrop-*like* experience over a protocol both sides can
actually speak.

## 2. Candidate protocols that work with Android

### Google Quick Share (formerly Nearby Share)

Quick Share is Android's built-in AirDrop. Open re-implementations: [NearDrop][neardrop]
(macOS), [rquickshare][rquickshare] (Rust/Tauri, Linux and macOS, GPLv3) and
[Packet][packet] (Rust, GTK4, GPLv3, built on NearDrop's protocol work).

How it works on Wi-Fi LAN ([NearDrop's protocol notes][neardrop-proto]):

- **Discovery.** mDNS service `_FC9F5ED42C8A._tcp`. The service name encodes an endpoint ID, and
  the TXT record carries the device name and type. Android only starts looking after it sees a
  **BLE advertisement** with service UUID `0xFE2C`. That is why the Linux apps need Bluetooth on
  for discovery.
- **Security.** A UKEY2 handshake (P-256 ECDH) derives keys with HKDF. Every frame is
  AES-256-CBC plus HMAC-SHA256, and a 4-digit PIN is shown for verification.
- **Framing.** Length-prefixed protobuf "offline frames". Payloads arrive as chunked `BYTES`/`FILE`
  transfers, with keep-alives every 10 s.

| Pros | Cons |
|---|---|
| Nothing to install on Android: it appears in the native share sheet | Reverse-engineered and partial. Parts of the handshake are still not understood, and Google can change it at any time |
| | Needs Bluetooth LE on the PC for reliable discovery |
| | Files only: no clipboard or text-message channel |
| | Users report the Linux ports work on some PCs and not on others |

### LocalSend

[LocalSend][localsend] has an **openly documented** protocol ([v2 spec][ls-proto]):

- **Discovery.** A JSON announcement goes to UDP multicast `224.0.0.167:53317` (`announce: true`).
  Everyone who hears it answers with `POST /api/localsend/v2/register` to the announcer. If
  multicast is blocked, clients fall back to probing every IP on the subnet.
- **Transfer.** The sender calls `POST /prepare-upload` with the file metadata. The receiver
  accepts or declines, then the sender calls `POST /upload?sessionId&fileId&token` once per file.
  There is also `/cancel`. A short text is sent as one `text/plain` "file" whose content is in
  `preview` (the receiver answers `204`).
- **Security.** Every device has a self-signed certificate. Its identity ("fingerprint") is the
  **uppercase hex SHA-256 of the certificate DER**. From reading LocalSend's current Rust core:
  - Clients **pin** the expected fingerprint during the TLS handshake.
  - Servers require a **TLS client certificate** whose fingerprint must match what the client
    claims in `register` and `prepare-upload`.
  - Certificates are RSA-2048, self-signed, checked for validity and signature.
- Apps exist for Android (Play Store and F-Droid), iOS, Windows, macOS and Linux.

| Pros | Cons |
|---|---|
| Documented and stable, with a real-world reference implementation to test against | Both sides must run a LocalSend-protocol app |
| Plain Wi-Fi LAN: no Bluetooth, root or special hardware | |
| Works with every LocalSend app on every platform | |

### Others

| Project | Approach | Why not the base |
|---|---|---|
| [KDE Connect][kdeconnect] / GSConnect | Its own TLS protocol (ports 1714–1764) with pairing. Covers notifications, SMS, clipboard and media control. | Much broader scope than AirDrop. File sharing is a side feature, not the AirDrop "tap a device" flow. |
| [PairDrop][pairdrop] / Snapdrop | Browser to browser over WebRTC | Needs a signaling server, and the page must be open on both sides. |

## 3. Decision

AirBridge speaks **LocalSend protocol v2**:

- It works on any Wi-Fi network both devices share, with no Bluetooth, root or special chips.
- It is **interoperable**. The Linux app works with the official LocalSend Android app, and the
  AirBridge Android app works with LocalSend on Windows, macOS and iOS.
- It is documented, and there is a reference implementation to test against.

What AirBridge adds on top:

- An AirDrop-style **radar UI with real animations** on both platforms: orbiting device bubbles,
  pulse rings, particle streams during transfers, progress rings and confetti.
- **Desktop integration:** a tray icon, notifications with Accept/Decline buttons, and a CLI
  (`airbridge send file --to phone`) that file managers and scripts can call.
- **Trusted devices:** your own phone is accepted without prompts, and only trusted devices may
  write to your clipboard.
- **Clipboard sharing** in both directions, a Quick Settings tile and a share-sheet target on
  Android.
- A **browser companion**. A phone without any app scans a QR code shown on the laptop, and the
  pairing secret travels in the URL fragment.

### Interop verification

The Linux app was tested against **LocalSend's own Rust CLI** (`localsend-cli`, built from the
official repository):

| Direction | Result |
|---|---|
| LocalSend → AirBridge | Discovery, mutual TLS, file transfer (byte-identical) and text message all work. |
| AirBridge → LocalSend | Discovery and file transfer accepted in LocalSend's UI (byte-identical). |

### Possible future work

- Optional **Quick Share receive mode** (like Packet or rquickshare), so a stock Android phone
  could send to Linux with no app installed.
- Resumable transfers, IPv6 multicast (`ff12::fb`-style groups, as newer LocalSend versions use),
  and protocol v3 once it is published.

[opendrop]: https://github.com/seemoo-lab/opendrop
[owl]: https://github.com/seemoo-lab/owl
[neardrop]: https://github.com/grishka/NearDrop
[neardrop-proto]: https://github.com/grishka/NearDrop/blob/master/PROTOCOL.md
[rquickshare]: https://github.com/Martichou/rquickshare
[packet]: https://github.com/nozwock/packet
[localsend]: https://github.com/localsend/localsend
[ls-proto]: https://github.com/localsend/protocol
[kdeconnect]: https://kdeconnect.kde.org/
[pairdrop]: https://github.com/schlagmichdoch/PairDrop
