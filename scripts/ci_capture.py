#!/usr/bin/env python3
"""Run the real NoteStrike window in --capture mode and check the PNGs.

Usage: ci_capture.py NOTESTRIKE_BINARY OUT_DIR

Captures the editor with a fixture file in the light and dark themes (the
theme is set through a throwaway settings.json; NOTESTRIKE_DATA_DIR keeps
the run away from the user's real settings and recovery journal), plus the
find/replace bar. Each PNG must be complete, at least the requested size
(HiDPI screens give larger images), and show real content. Exits non-zero
on the first failure. Used by .github/workflows/native-editor.yml on
Windows, Linux (under xvfb-run) and macOS. Standard library only.
"""

import json
import os
import pathlib
import struct
import subprocess
import sys
import zlib

WIDTH, HEIGHT = 960, 640
TIMEOUT_SECONDS = 120
FIXTURE = "Project Hub\n\nReview priorities\nCapture decisions\nFollow up on open tasks\n"


def png_report(path: pathlib.Path) -> str:
    data = path.read_bytes()
    if len(data) < 5000 or not data.startswith(b"\x89PNG\r\n\x1a\n"):
        raise SystemExit(f"{path.name}: capture is missing or incomplete ({len(data)} bytes)")
    width, height = struct.unpack(">II", data[16:24])
    if width < WIDTH or height < HEIGHT:
        raise SystemExit(f"{path.name}: unexpected dimensions {width}x{height}")
    offset, idat = 8, bytearray()
    while offset < len(data):
        length = struct.unpack(">I", data[offset : offset + 4])[0]
        if data[offset + 4 : offset + 8] == b"IDAT":
            idat.extend(data[offset + 8 : offset + 8 + length])
        offset += length + 12
    distinct = len(set(zlib.decompress(idat)))
    if distinct < 24:
        raise SystemExit(f"{path.name}: capture lacks visible content ({distinct} byte values)")
    return f"{path.name}: {width}x{height}, {len(data)} bytes"


def capture(binary: pathlib.Path, out_dir: pathlib.Path, name: str, theme: str, extra: list) -> None:
    data_dir = out_dir / f"data-{name}"
    data_dir.mkdir(parents=True, exist_ok=True)
    (data_dir / "settings.json").write_text(json.dumps({"theme": theme}), encoding="utf-8")
    fixture = out_dir / "fixture.txt"
    png = out_dir / f"notestrike-{name}.png"
    command = [str(binary), "--capture", str(png), str(WIDTH), str(HEIGHT), *extra, str(fixture)]
    env = dict(os.environ, NOTESTRIKE_DATA_DIR=str(data_dir))
    print("+", " ".join(command), flush=True)
    result = subprocess.run(command, env=env, timeout=TIMEOUT_SECONDS, capture_output=True, text=True)
    if result.stdout:
        print(result.stdout, end="")
    if result.stderr:
        print(result.stderr, end="", file=sys.stderr)
    if result.returncode != 0:
        raise SystemExit(f"{name}: notestrike exited with {result.returncode}")
    print(png_report(png), flush=True)


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    binary = pathlib.Path(sys.argv[1]).resolve()
    out_dir = pathlib.Path(sys.argv[2]).resolve()
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "fixture.txt").write_text(FIXTURE, encoding="utf-8", newline="\n")
    capture(binary, out_dir, "editor-light", "light", [])
    capture(binary, out_dir, "editor-dark", "dark", [])
    capture(binary, out_dir, "find-replace-light", "light", ["--find", "priorities", "plans"])


if __name__ == "__main__":
    main()
