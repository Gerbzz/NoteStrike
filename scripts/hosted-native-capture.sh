#!/usr/bin/env bash
set -euo pipefail
probe_root="${RUNNER_TEMP}/notestrike-editor-visual"
notebook="${probe_root}/notebook"
mkdir -p "$probe_root"
cli="./target/debug/notestrike"
native="./target/debug/notestrike-native"
"$cli" init "$notebook"
hub_body=
if [[ ! "$hub" =~ ^[0-9a-f-]{36}$ ]]; then echo 'Fixture hub creation failed.' >&2; exit 1; fi
for index in $(seq -w 1 12); do
  "$cli" new "$notebook" "B Review $index" "# B Review $index"$'\n\n'"See [[$hub|Project Hub]] for the running plan."$'\n\nA short note for the next review.' >/dev/null
done
backlinks=$("$cli" backlinks "$notebook" "$hub" | wc -l)
if [[ "$backlinks" -ne 12 ]]; then echo "Expected 12 backlinks; found $backlinks" >&2; exit 1; fi
echo "FIXTURE_BACKLINKS:$backlinks"
export LIBGL_ALWAYS_SOFTWARE=1
export GALLIUM_DRIVER=llvmpipe
for spec in WIDE:1280:800 COMPACT:800:520; do
  IFS=: read -r label width height <<< "$spec"
  capture="${probe_root}/${label,,}.png"
  stdout="${probe_root}/${label,,}.out.txt"
  stderr="${probe_root}/${label,,}.err.txt"
  capture_flag="--capture"\n  if [[ "$label" == "COMPACT" ]]; then capture_flag="--capture-scroll-end"; fi\n  if ! timeout 50s xvfb-run -a -s '-screen 0 1600x1000x24' "$native" "$notebook" "$capture_flag" "$capture" "$width" "$height" >"$stdout" 2>"$stderr"; then
    echo "$label stdout:"; cat "$stdout"
    echo "$label stderr:"; cat "$stderr"
    echo "Hosted $label native capture failed." >&2
    exit 1
  fi
  python3 - "$capture" "$label" "$width" "$height" <<'PY'
import base64
import hashlib
import pathlib
import struct
import sys
import zlib
path = pathlib.Path(sys.argv[1])
label = sys.argv[2]
minimum_width = int(sys.argv[3])
minimum_height = int(sys.argv[4])
data = path.read_bytes()
if len(data) < 5000 or not data.startswith(b"\x89PNG\r\n\x1a\n") or not data.endswith(b"IEND\xaeB\x60\x82"):
    raise SystemExit(f"{label} capture is incomplete or blank-sized.")
width, height, depth, color = struct.unpack(">IIBB", data[16:26])
if width < minimum_width or height < minimum_height or depth != 8 or color not in (2, 6):
    raise SystemExit(f"{label} unexpected PNG image: {width}x{height}, depth {depth}, color {color}.")
offset = 8
idat = bytearray()
while offset < len(data):
    length = struct.unpack(">I", data[offset:offset + 4])[0]
    kind = data[offset + 4:offset + 8]
    chunk = data[offset + 8:offset + 8 + length]
    if kind == b"IDAT":
        idat.extend(chunk)
    offset += 12 + length
raw = zlib.decompress(idat)
if len(raw) < width * height * 3 or len(set(raw)) < 24:
    raise SystemExit(f"{label} decoded pixels lack expected content.")
print(f"{label}_DIMENSIONS:{width}x{height}")
print(f"{label}_RAW_VARIANCE:{len(set(raw))}")
print(f"{label}_SHA256:{hashlib.sha256(data).hexdigest().upper()}")
print(f"{label}_BYTES:{len(data)}")
encoded = base64.b64encode(data).decode("ascii")
for start in range(0, len(encoded), 4000):
    print(f"{label}_CHUNK:{encoded[start:start+4000]}")
PY
done
# Project Hub\n\nPlan the next release with a clean, local notebook.\n\n- Review priorities\n- Capture decisions\n- Follow up on open tasks\n\n## Review log\n'\nfor item in $(seq -w 1 60); do\n  hub_body+="Review item $item: capture the next decision."
if [[ ! "$hub" =~ ^[0-9a-f-]{36}$ ]]; then echo 'Fixture hub creation failed.' >&2; exit 1; fi
for index in $(seq -w 1 12); do
  "$cli" new "$notebook" "B Review $index" "# B Review $index"$'\n\n'"See [[$hub|Project Hub]] for the running plan."$'\n\nA short note for the next review.' >/dev/null
done
backlinks=$("$cli" backlinks "$notebook" "$hub" | wc -l)
if [[ "$backlinks" -ne 12 ]]; then echo "Expected 12 backlinks; found $backlinks" >&2; exit 1; fi
echo "FIXTURE_BACKLINKS:$backlinks"
export LIBGL_ALWAYS_SOFTWARE=1
export GALLIUM_DRIVER=llvmpipe
for spec in WIDE:1280:800 COMPACT:800:520; do
  IFS=: read -r label width height <<< "$spec"
  capture="${probe_root}/${label,,}.png"
  stdout="${probe_root}/${label,,}.out.txt"
  stderr="${probe_root}/${label,,}.err.txt"
  if ! timeout 50s xvfb-run -a -s '-screen 0 1600x1000x24' "$native" "$notebook" --capture "$capture" "$width" "$height" >"$stdout" 2>"$stderr"; then
    echo "$label stdout:"; cat "$stdout"
    echo "$label stderr:"; cat "$stderr"
    echo "Hosted $label native capture failed." >&2
    exit 1
  fi
  python3 - "$capture" "$label" "$width" "$height" <<'PY'
import base64
import hashlib
import pathlib
import struct
import sys
import zlib
path = pathlib.Path(sys.argv[1])
label = sys.argv[2]
minimum_width = int(sys.argv[3])
minimum_height = int(sys.argv[4])
data = path.read_bytes()
if len(data) < 5000 or not data.startswith(b"\x89PNG\r\n\x1a\n") or not data.endswith(b"IEND\xaeB\x60\x82"):
    raise SystemExit(f"{label} capture is incomplete or blank-sized.")
width, height, depth, color = struct.unpack(">IIBB", data[16:26])
if width < minimum_width or height < minimum_height or depth != 8 or color not in (2, 6):
    raise SystemExit(f"{label} unexpected PNG image: {width}x{height}, depth {depth}, color {color}.")
offset = 8
idat = bytearray()
while offset < len(data):
    length = struct.unpack(">I", data[offset:offset + 4])[0]
    kind = data[offset + 4:offset + 8]
    chunk = data[offset + 8:offset + 8 + length]
    if kind == b"IDAT":
        idat.extend(chunk)
    offset += 12 + length
raw = zlib.decompress(idat)
if len(raw) < width * height * 3 or len(set(raw)) < 24:
    raise SystemExit(f"{label} decoded pixels lack expected content.")
print(f"{label}_DIMENSIONS:{width}x{height}")
print(f"{label}_RAW_VARIANCE:{len(set(raw))}")
print(f"{label}_SHA256:{hashlib.sha256(data).hexdigest().upper()}")
print(f"{label}_BYTES:{len(data)}")
encoded = base64.b64encode(data).decode("ascii")
for start in range(0, len(encoded), 4000):
    print(f"{label}_CHUNK:{encoded[start:start+4000]}")
PY
done
\\n'\ndone\nhub_body+=
if [[ ! "$hub" =~ ^[0-9a-f-]{36}$ ]]; then echo 'Fixture hub creation failed.' >&2; exit 1; fi
for index in $(seq -w 1 12); do
  "$cli" new "$notebook" "B Review $index" "# B Review $index"$'\n\n'"See [[$hub|Project Hub]] for the running plan."$'\n\nA short note for the next review.' >/dev/null
done
backlinks=$("$cli" backlinks "$notebook" "$hub" | wc -l)
if [[ "$backlinks" -ne 12 ]]; then echo "Expected 12 backlinks; found $backlinks" >&2; exit 1; fi
echo "FIXTURE_BACKLINKS:$backlinks"
export LIBGL_ALWAYS_SOFTWARE=1
export GALLIUM_DRIVER=llvmpipe
for spec in WIDE:1280:800 COMPACT:800:520; do
  IFS=: read -r label width height <<< "$spec"
  capture="${probe_root}/${label,,}.png"
  stdout="${probe_root}/${label,,}.out.txt"
  stderr="${probe_root}/${label,,}.err.txt"
  if ! timeout 50s xvfb-run -a -s '-screen 0 1600x1000x24' "$native" "$notebook" --capture "$capture" "$width" "$height" >"$stdout" 2>"$stderr"; then
    echo "$label stdout:"; cat "$stdout"
    echo "$label stderr:"; cat "$stderr"
    echo "Hosted $label native capture failed." >&2
    exit 1
  fi
  python3 - "$capture" "$label" "$width" "$height" <<'PY'
import base64
import hashlib
import pathlib
import struct
import sys
import zlib
path = pathlib.Path(sys.argv[1])
label = sys.argv[2]
minimum_width = int(sys.argv[3])
minimum_height = int(sys.argv[4])
data = path.read_bytes()
if len(data) < 5000 or not data.startswith(b"\x89PNG\r\n\x1a\n") or not data.endswith(b"IEND\xaeB\x60\x82"):
    raise SystemExit(f"{label} capture is incomplete or blank-sized.")
width, height, depth, color = struct.unpack(">IIBB", data[16:26])
if width < minimum_width or height < minimum_height or depth != 8 or color not in (2, 6):
    raise SystemExit(f"{label} unexpected PNG image: {width}x{height}, depth {depth}, color {color}.")
offset = 8
idat = bytearray()
while offset < len(data):
    length = struct.unpack(">I", data[offset:offset + 4])[0]
    kind = data[offset + 4:offset + 8]
    chunk = data[offset + 8:offset + 8 + length]
    if kind == b"IDAT":
        idat.extend(chunk)
    offset += 12 + length
raw = zlib.decompress(idat)
if len(raw) < width * height * 3 or len(set(raw)) < 24:
    raise SystemExit(f"{label} decoded pixels lack expected content.")
print(f"{label}_DIMENSIONS:{width}x{height}")
print(f"{label}_RAW_VARIANCE:{len(set(raw))}")
print(f"{label}_SHA256:{hashlib.sha256(data).hexdigest().upper()}")
print(f"{label}_BYTES:{len(data)}")
encoded = base64.b64encode(data).decode("ascii")
for start in range(0, len(encoded), 4000):
    print(f"{label}_CHUNK:{encoded[start:start+4000]}")
PY
done
\\nFINAL ACCESSIBLE LINE: save and close the review.'\nhub=$("$cli" new "$notebook" "A Project Hub" "$hub_body")
if [[ ! "$hub" =~ ^[0-9a-f-]{36}$ ]]; then echo 'Fixture hub creation failed.' >&2; exit 1; fi
for index in $(seq -w 1 12); do
  "$cli" new "$notebook" "B Review $index" "# B Review $index"$'\n\n'"See [[$hub|Project Hub]] for the running plan."$'\n\nA short note for the next review.' >/dev/null
done
backlinks=$("$cli" backlinks "$notebook" "$hub" | wc -l)
if [[ "$backlinks" -ne 12 ]]; then echo "Expected 12 backlinks; found $backlinks" >&2; exit 1; fi
echo "FIXTURE_BACKLINKS:$backlinks"
export LIBGL_ALWAYS_SOFTWARE=1
export GALLIUM_DRIVER=llvmpipe
for spec in WIDE:1280:800 COMPACT:800:520; do
  IFS=: read -r label width height <<< "$spec"
  capture="${probe_root}/${label,,}.png"
  stdout="${probe_root}/${label,,}.out.txt"
  stderr="${probe_root}/${label,,}.err.txt"
  if ! timeout 50s xvfb-run -a -s '-screen 0 1600x1000x24' "$native" "$notebook" --capture "$capture" "$width" "$height" >"$stdout" 2>"$stderr"; then
    echo "$label stdout:"; cat "$stdout"
    echo "$label stderr:"; cat "$stderr"
    echo "Hosted $label native capture failed." >&2
    exit 1
  fi
  python3 - "$capture" "$label" "$width" "$height" <<'PY'
import base64
import hashlib
import pathlib
import struct
import sys
import zlib
path = pathlib.Path(sys.argv[1])
label = sys.argv[2]
minimum_width = int(sys.argv[3])
minimum_height = int(sys.argv[4])
data = path.read_bytes()
if len(data) < 5000 or not data.startswith(b"\x89PNG\r\n\x1a\n") or not data.endswith(b"IEND\xaeB\x60\x82"):
    raise SystemExit(f"{label} capture is incomplete or blank-sized.")
width, height, depth, color = struct.unpack(">IIBB", data[16:26])
if width < minimum_width or height < minimum_height or depth != 8 or color not in (2, 6):
    raise SystemExit(f"{label} unexpected PNG image: {width}x{height}, depth {depth}, color {color}.")
offset = 8
idat = bytearray()
while offset < len(data):
    length = struct.unpack(">I", data[offset:offset + 4])[0]
    kind = data[offset + 4:offset + 8]
    chunk = data[offset + 8:offset + 8 + length]
    if kind == b"IDAT":
        idat.extend(chunk)
    offset += 12 + length
raw = zlib.decompress(idat)
if len(raw) < width * height * 3 or len(set(raw)) < 24:
    raise SystemExit(f"{label} decoded pixels lack expected content.")
print(f"{label}_DIMENSIONS:{width}x{height}")
print(f"{label}_RAW_VARIANCE:{len(set(raw))}")
print(f"{label}_SHA256:{hashlib.sha256(data).hexdigest().upper()}")
print(f"{label}_BYTES:{len(data)}")
encoded = base64.b64encode(data).decode("ascii")
for start in range(0, len(encoded), 4000):
    print(f"{label}_CHUNK:{encoded[start:start+4000]}")
PY
done
