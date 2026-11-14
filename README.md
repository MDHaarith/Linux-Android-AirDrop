# AirBridge

AirDrop-style sharing between Linux, Windows and Android. Files, folders, photos, text and the
clipboard go straight from device to device over your own network. No cloud, no cable, no account.

## Install

Download from the newest [release](https://github.com/MDHaarith/AirBridge/releases/latest):

- **Android 10+:** `airbridge-android.apk`. Allow installs from your browser or file manager.
- **Windows 10/11 (x64):** `airbridge-setup.exe`, or `airbridge.exe` without installing. Allow it
  through Windows Firewall on private networks.
- **Linux (x86_64):** `airbridge-x86_64.AppImage`. Run `chmod +x` on it, then start it. Needs GTK 3
  and WebKitGTK 4.1.

Then scan the desktop app's **QR** code from the Android app's **Pair with QR Code**. All apps
update themselves from the releases here.

Windows SmartScreen may say "Windows protected your PC" (click **More info**, then **Run
anyway**), and Play Protect may warn on Android (tap **More details**, then **Install anyway**).
The apps are not code-signed with a paid certificate yet. These warnings do not mean a virus was
found.

Open ports if a firewall is on: 53317 TCP+UDP (LocalSend), 53318 TCP (browser page),
53319 TCP and 5353 UDP (Quick Share).

## What it does

- Finds nearby devices automatically on the same Wi-Fi, and paired devices over Tailscale or any VPN.
- Sends files of any size, whole folders, photos with previews, text, links and the clipboard, both ways.
- Asks before receiving, with a photo preview. Your own trusted devices are accepted without asking.
- Speaks the LocalSend v2 protocol, so it works with the official LocalSend apps (including iPhone and Mac).
- Speaks Google Quick Share on Linux and Windows, so a stock Android phone can share with no app installed.
- Serves a browser page, so a phone with no app at all can send and receive after scanning a QR code.
- Encrypts everything with TLS and pinned certificates. Updates are signed (Ed25519).

Received files go to `Downloads/AirBridge` on Android and to your Downloads folder on Windows and
Linux.

## Repository layout

```
linux/      Rust service, CLI and embedded web UI for Linux and Windows (ui/ is TypeScript)
android/    Kotlin + Jetpack Compose app; core/ is the protocol engine, ui/ the screens
docs/       SECURITY.md, RESEARCH.md, ROADMAP.md
```

## Build

Linux (Rust 1.89 or newer):

```bash
cd linux
cargo build --release --features webview,edge   # needs WebKitGTK 4.1, GTK 3, gtk-layer-shell
./packaging/install.sh                          # optional: menu entry, autostart, file manager action
```

Windows: install Rust with the MSVC toolchain, then run `cargo build --release --locked` in `linux/`.

Android (JDK 21):

```bash
cd android
./gradlew assembleRelease
```

After editing a `.ts` file in `linux/ui/src/`, run `npm ci && npm run build` in `linux/ui/`. The
compiled `.js` files are committed, and CI checks that they match.

## Checks

```bash
python3 -m unittest discover -s .github/scripts -p 'test_*.py'
cd linux && cargo test && cargo clippy --all-targets -- -D warnings
cd android && ./gradlew testDebugUnitTest verifyPaparazziDebug
```

## Releases

Every successful CI run on `main` triggers the **Release** workflow. It picks the next patch
version, builds the Android APK, the Windows installer and exe, and the Linux AppImage, and
publishes them as a release here, where the apps look for updates. To choose a version, run
**Release** from the Actions tab or push a `vX.Y.Z` tag.

Required secrets: `UPDATE_SIGNING_KEY`, `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`.

## License

MIT. See [LICENSE](LICENSE). Not affiliated with Apple, Google or the LocalSend project.
