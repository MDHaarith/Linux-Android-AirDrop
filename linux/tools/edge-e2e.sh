#!/usr/bin/env bash
# End-to-end check of the screen-edge drop zone under Xvfb (X11):
# a drag source offers an image like Firefox does, xdotool drags it to the
# right edge and drops it on a device; screenshots go to $OUT.
#
#   cargo build --features edge --examples && tools/edge-e2e.sh /tmp/edge-shots
set -euo pipefail
cd "$(dirname "$0")/.."
OUT=${1:-/tmp/edge-shots}
BIN=$PWD/target/debug/airbridge
SOURCE=$PWD/target/debug/examples/edge_drag_source
WORK=$(mktemp -d)
mkdir -p "$OUT" "$WORK/a/config" "$WORK/b/config"
export DISPLAY=:77 NO_COLOR=1
pids=()
cleanup() { kill "${pids[@]}" 2>/dev/null || true; wait 2>/dev/null || true; }
trap cleanup EXIT

Xvfb :77 -screen 0 1280x800x24 -nolisten tcp >/dev/null 2>&1 & pids+=($!)
sleep 1
if command -v picom >/dev/null; then picom --backend xrender --no-fading-openclose >/dev/null 2>&1 & pids+=($!); else xcompmgr -c -r 0 -o 0 >/dev/null 2>&1 & pids+=($!); fi
# Something to look at behind the panel.
convert -size 1280x800 gradient:'#5d8cc8'-'#36548c' "$WORK/bg.png"
if command -v hsetroot >/dev/null; then hsetroot -fill "$WORK/bg.png" >/dev/null 2>&1; fi

echo '{"alias":"Pixel 8","notifications":false,"port":54610,"webPort":54611,"webCompanion":false,"edgeDrop":false,"quickShare":false}' > "$WORK/a/config/settings.json"
echo '{"alias":"ThinkPad","notifications":false,"port":54620,"webPort":54621,"webCompanion":false,"quickShare":false}' > "$WORK/b/config/settings.json"
AIRBRIDGE_HOME=$WORK/a "$BIN" daemon > "$OUT/receiver.log" 2>&1 & pids+=($!)
AIRBRIDGE_HOME=$WORK/b AIRBRIDGE_LOG=debug,zbus=warn,hyper=info,rustls=warn "$BIN" daemon > "$OUT/sender.log" 2>&1 & pids+=($!)
for _ in $(seq 60); do grep -q "Drop zone ready" "$OUT/sender.log" && grep -q "Found Pixel 8" "$OUT/sender.log" && break; sleep 0.5; done
grep "Drop zone ready" "$OUT/sender.log"

convert -size 480x320 gradient:'#3e68ba'-'#fab278' "$WORK/photo.png"
"$SOURCE" image "$WORK/photo.png" "https://images.example.com/gallery/sunset.png?w=480" & pids+=($!)
sleep 1.5

shot() { import -window root "$OUT/$1.png"; }
xdotool mousemove 170 110 mousedown 1
for x in $(seq 180 60 1260); do xdotool mousemove "$x" 400; sleep 0.03; done
xdotool mousemove 1279 400; sleep 0.9; shot 1-panel-open
xdotool mousemove 1180 420; sleep 0.1; xdotool mousemove 1120 429; sleep 0.6; shot 2-hover
xdotool mouseup 1; sleep 0.15; shot 3-dropped
sleep 0.8; shot 4-waiting

read -r PORT TOKEN < <(python3 -c "import json;d=json.load(open('$WORK/a/run/instance.json'));print(d['port'],d['token'])")
ID=$(curl -s -H "x-airbridge-token: $TOKEN" "http://127.0.0.1:$PORT/api/state" | python3 -c "import json,sys;print([t['id'] for t in json.load(sys.stdin)['transfers'] if t['status']=='waiting'][0])")
curl -s -X POST -H "x-airbridge-token: $TOKEN" "http://127.0.0.1:$PORT/api/transfers/$ID/accept" >/dev/null
sleep 0.55; shot 5-sent
sleep 3; shot 6-closed

ls -la "$WORK/a/downloads"
cmp "$WORK/photo.png" "$WORK/a/downloads/sunset.png" && echo "RECEIVED: sunset.png is identical"
