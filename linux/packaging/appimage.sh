#!/usr/bin/env bash
# Packs a release binary into airbridge-x86_64.AppImage.
#   ./appimage.sh path/to/airbridge [output-dir]
# GTK, WebKitGTK 4.1 and gtk-layer-shell come from the system, as on any desktop.
set -euo pipefail

bin="$(realpath "$1")"
out="$(realpath "${2:-.}")"
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# Pinned tools, checked before use.
fetch() { # url sha256 dest
  curl -fsSL --proto '=https' --retry 3 -o "$3" "$1"
  echo "$2  $3" | sha256sum -c --quiet
}
fetch https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage \
  ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0 "$work/appimagetool"
fetch https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64 \
  2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d "$work/runtime"
chmod +x "$work/appimagetool"

app="$work/AirBridge.AppDir"
install -Dm755 "$bin" "$app/usr/bin/airbridge"
install -Dm644 "$here/../ui/icon.svg" "$app/airbridge.svg"
install -Dm644 "$here/../ui/icon.svg" "$app/usr/share/icons/hicolor/scalable/apps/airbridge.svg"
install -Dm644 "$here/airbridge.desktop" "$app/airbridge.desktop"
cat > "$app/AppRun" <<'EOF'
#!/bin/sh
exec "$(dirname "$(readlink -f "$0")")/usr/bin/airbridge" "$@"
EOF
chmod +x "$app/AppRun"

# No FUSE on CI runners.
APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 "$work/appimagetool" --no-appstream \
  --runtime-file "$work/runtime" "$app" "$out/airbridge-x86_64.AppImage"
