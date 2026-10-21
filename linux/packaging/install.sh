#!/usr/bin/env bash
# Builds AirBridge and installs it for the current user (no root needed).
#   ./install.sh             build + install
#   ./install.sh --uninstall remove everything this script installed
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
crate="$(dirname "$here")"
bin_dir="${HOME}/.local/bin"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
config="${XDG_CONFIG_HOME:-$HOME/.config}"

files=(
  "$bin_dir/airbridge"
  "$data/applications/airbridge.desktop"
  "$data/icons/hicolor/scalable/apps/airbridge.svg"
  "$config/autostart/airbridge.desktop"
  "$data/nautilus/scripts/Send with AirBridge"
  "$data/kio/servicemenus/airbridge.desktop"
  "$data/nemo/actions/airbridge.nemo_action"
)

if [[ "${1:-}" == "--uninstall" ]]; then
  "$bin_dir/airbridge" quit 2>/dev/null || true
  rm -f "${files[@]}"
  echo "AirBridge removed. Settings and history remain in $config/airbridge and $data/airbridge."
  exit 0
fi

if ! command -v cargo >/dev/null; then
  echo "Rust is required: https://rustup.rs (then re-run this script)" >&2
  exit 1
fi

features=()
if pkg-config --exists webkit2gtk-4.1 gtk+-3.0 2>/dev/null; then
  features+=(webview)
  echo "→ Building with a native window (WebKitGTK found)"
else
  echo "→ Building without the native window (install libwebkit2gtk-4.1-dev for it);"
  echo "  AirBridge will open in a Chromium app window or your browser instead."
fi
if pkg-config --exists gtk+-3.0 gtk-layer-shell-0 2>/dev/null; then
  features+=(edge)
  echo "→ Building with the screen-edge drop zone (gtk-layer-shell found)"
else
  echo "→ Building without the screen-edge drop zone (install libgtk-layer-shell-dev"
  echo "  (Debian/Ubuntu) or gtk-layer-shell (Fedora, Arch) for it)."
fi

feature_args=()
if [ ${#features[@]} -gt 0 ]; then
  feature_args=(--features "$(IFS=,; echo "${features[*]}")")
fi
(cd "$crate" && cargo build --release --locked "${feature_args[@]}")

install -Dm755 "$crate/target/release/airbridge" "$bin_dir/airbridge"
install -Dm644 "$crate/ui/icon.svg" "$data/icons/hicolor/scalable/apps/airbridge.svg"
install -Dm644 "$here/airbridge.desktop" "$data/applications/airbridge.desktop"
install -Dm644 "$here/airbridge-autostart.desktop" "$config/autostart/airbridge.desktop"
install -Dm755 "$here/nautilus-send-to-phone.sh" "$data/nautilus/scripts/Send with AirBridge"
install -Dm644 "$here/dolphin-airbridge.desktop" "$data/kio/servicemenus/airbridge.desktop"
install -Dm644 "$here/nemo-airbridge.nemo_action" "$data/nemo/actions/airbridge.nemo_action"
command -v update-desktop-database >/dev/null && update-desktop-database "$data/applications" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q "$data/icons/hicolor" 2>/dev/null || true

echo
echo "✓ Installed to $bin_dir/airbridge (make sure it is on your PATH)"
echo "  Starts automatically at login; launch it now from your app menu or run: airbridge"
echo

# Firewalls commonly block discovery; show the exact commands instead of running sudo.
if command -v ufw >/dev/null && ufw status 2>/dev/null | grep -q "Status: active"; then
  echo "ufw is active. Allow AirBridge with:"
  echo "  sudo ufw allow 53317/tcp && sudo ufw allow 53317/udp && sudo ufw allow 53318/tcp"
  echo "  sudo ufw allow 53319/tcp && sudo ufw allow 5353/udp   # Quick Share"
elif command -v firewall-cmd >/dev/null && firewall-cmd --state >/dev/null 2>&1; then
  echo "firewalld is active. Allow AirBridge with:"
  echo "  sudo firewall-cmd --permanent --add-port=53317/tcp --add-port=53317/udp --add-port=53318/tcp --add-port=53319/tcp --add-service=mdns && sudo firewall-cmd --reload"
fi
