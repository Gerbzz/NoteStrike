"""Read-only validation for original SVGs, transparent PNGs and packaged Windows icons."""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import struct
import xml.etree.ElementTree as ET
import zlib


def digest(data):
    return hashlib.sha256(data).hexdigest()


def check_png(data):
    assert data[:8] == b"\x89PNG\r\n\x1a\n", "PNG signature"
    at, pixels, width, height = 8, bytearray(), 0, 0
    while at < len(data):
        size = struct.unpack_from(">I", data, at)[0]
        kind, body = data[at+4:at+8], data[at+8:at+8+size]
        assert zlib.crc32(kind+body) & 0xffffffff == struct.unpack_from(">I", data, at+8+size)[0], "PNG CRC"
        if kind == b"IHDR":
            width, height, depth, color, compression, filtering, interlace = struct.unpack(">IIBBBBB", body)
            assert depth == 8 and color == 6 and compression == filtering == interlace == 0, "RGBA8 PNG"
            assert width == height and 0 < width <= 2048, "square bounded PNG"
        if kind == b"IDAT":
            pixels.extend(body)
        at += size + 12
    raw, stride, rows, previous = zlib.decompress(pixels), width*4, [], bytearray(width*4)
    assert len(raw) == (stride+1)*height, "PNG data size"
    for y in range(height):
        mode = raw[y*(stride+1)]
        row = bytearray(raw[y*(stride+1)+1:(y+1)*(stride+1)])
        assert 0 <= mode <= 4, "PNG row filter"
        for i in range(stride):
            left, up, upper_left = row[i-4] if i >= 4 else 0, previous[i], previous[i-4] if i >= 4 else 0
            if mode == 1:
                row[i] = (row[i]+left) & 255
            elif mode == 2:
                row[i] = (row[i]+up) & 255
            elif mode == 3:
                row[i] = (row[i]+((left+up)//2)) & 255
            elif mode == 4:
                p = left+up-upper_left
                distances = [abs(p-left), abs(p-up), abs(p-upper_left)]
                predictor = [left, up, upper_left][distances.index(min(distances))]
                row[i] = (row[i]+predictor) & 255
        rows.append(row)
        previous = row
    assert all(rows[y][x*4+3] == 0 for x, y in [(0, 0), (width-1, 0), (0, height-1), (width-1, height-1)]), "transparent corners"
    visible = [(x, y) for y, row in enumerate(rows) for x in range(width) if row[x*4+3]]
    assert visible, "nonempty mark"
    xs, ys = zip(*visible)
    bounds = [min(xs), min(ys), max(xs)+1, max(ys)+1]
    assert abs(bounds[0]+bounds[2]-width) <= 1 and abs(bounds[1]+bounds[3]-height) <= 1, "centered alpha bounds"
    return {"size": width, "alpha_bounds": bounds, "sha256": digest(data)}


def ico_images(data):
    reserved, kind, count = struct.unpack_from("<HHH", data)
    assert reserved == 0 and kind == 1 and count == 7, "seven-size ICO"
    images = {}
    for i in range(count):
        w, h, palette, reserved, planes, bits, size, offset = struct.unpack_from("<BBBBHHII", data, 6+16*i)
        w, h = w or 256, h or 256
        assert w == h and planes == 1 and bits == 32 and offset+size <= len(data), "ICO directory"
        body = data[offset:offset+size]
        assert check_png(body)["size"] == w, "ICO dimensions"
        images[w] = body
    assert sorted(images) == [16, 24, 32, 48, 64, 128, 256], "ICO sizes"
    return images


def verify_group(group, images, read_icon):
    assert struct.unpack_from("<HHH", group) == (0, 1, 7), "embedded icon group"
    assert len(group) == 6+7*14, "embedded icon directory length"
    observed = set()
    for i in range(7):
        w, h, palette, reserved, planes, bits, size, resource_id = struct.unpack_from("<BBBBHHIH", group, 6+14*i)
        width, height = w or 256, h or 256
        assert width in images and width not in observed, "unique embedded sizes"
        assert height == width and palette == reserved == 0 and planes == 1 and bits == 32, "embedded icon metadata"
        body = read_icon(resource_id)
        assert size == len(body) == len(images[width]), "embedded icon payload length"
        assert body == images[width], "embedded icon PNG matches SVG render"
        observed.add(width)
    assert observed == set(images), "all embedded sizes present"
    return sorted(observed)


def pe_icons(exe, images):
    assert os.name == "nt", "PE resource inspection requires Windows"
    from ctypes import wintypes
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.LoadLibraryExW.argtypes, kernel.LoadLibraryExW.restype = [wintypes.LPCWSTR, wintypes.HANDLE, wintypes.DWORD], wintypes.HMODULE
    kernel.FindResourceW.argtypes, kernel.FindResourceW.restype = [wintypes.HMODULE, ctypes.c_void_p, ctypes.c_void_p], ctypes.c_void_p
    kernel.LoadResource.argtypes, kernel.LoadResource.restype = [wintypes.HMODULE, ctypes.c_void_p], ctypes.c_void_p
    kernel.LockResource.argtypes, kernel.LockResource.restype = [ctypes.c_void_p], ctypes.c_void_p
    kernel.SizeofResource.argtypes, kernel.SizeofResource.restype = [wintypes.HMODULE, ctypes.c_void_p], wintypes.DWORD
    kernel.FreeLibrary.argtypes, kernel.FreeLibrary.restype = [wintypes.HMODULE], wintypes.BOOL
    module = kernel.LoadLibraryExW(str(exe.resolve()), None, 2)  # data only; does not run application code
    assert module, "load executable resource table"
    try:
        def resource(name, kind):
            handle = kernel.FindResourceW(module, name, kind)
            assert handle, f"resource {kind}/{name} exists"
            loaded = kernel.LoadResource(module, handle)
            address = kernel.LockResource(loaded)
            assert address, "resource payload address"
            return ctypes.string_at(address, kernel.SizeofResource(module, handle))
        group = resource(1, 14)
        observed = verify_group(group, images, lambda resource_id: resource(resource_id, 3))
        return {"executable": str(exe), "embedded_sizes": observed, "resource_payloads_match": True}
    finally:
        kernel.FreeLibrary(module)


def main():
    parser = argparse.ArgumentParser()
    for field in ["svg", "png", "ico", "original", "original-sha256"]:
        parser.add_argument("--"+field, required=True)
    parser.add_argument("--exe")
    args = parser.parse_args()
    assert digest(Path(args.original).read_bytes()) == args.original_sha256.lower(), "historical original preserved"
    tree = ET.parse(args.svg)
    root = tree.getroot()
    assert root.attrib.get("viewBox") == "0 0 96 96", "master geometry space"
    assert all(e.tag.rsplit("}", 1)[-1] not in {"image", "text", "script", "foreignObject"} for e in tree.iter()), "geometry-only SVG"
    for element in tree.iter():
        assert not any(k.endswith("href") for k in element.attrib), "self-contained vector"
    png = check_png(Path(args.png).read_bytes())
    images = ico_images(Path(args.ico).read_bytes())
    result = {"png": png, "ico_sizes": sorted(images), "original_preserved": True}
    if args.exe:
        result["pe"] = pe_icons(Path(args.exe), images)
    print(json.dumps(result))


if __name__ == "__main__":
    main()
