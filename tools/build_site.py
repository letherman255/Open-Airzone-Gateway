#!/usr/bin/env python3
"""Build the GitHub Pages site: web flasher, ESP Web Tools manifest, OTA manifest, checksums.

Python standard library only.
"""
import argparse
import hashlib
import json
import os
import re
import shutil
import sys

FILL = b"\xff"


def _int(s):
    return int(s.strip(), 0)


def layout(csv_text):
    """Return {"otadata": (offset, size), "app": offset_of_first_app_partition}."""
    otadata = None
    app = None
    for line in csv_text.splitlines():
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        f = [x.strip() for x in line.split(",")]
        if len(f) < 5:
            continue
        _name, ptype, subtype, offset, size = f[:5]
        if ptype == "data" and subtype == "ota":
            otadata = (_int(offset), _int(size))
        elif ptype == "app" and app is None:
            app = _int(offset)
    if otadata is None or app is None:
        raise ValueError("partitions.csv lacks an otadata or an app partition")
    return {"otadata": otadata, "app": app}


def check_tag(tag, cargo_text):
    """Return the Cargo package version if it matches the tag, else raise ValueError."""
    m = re.search(r'^version\s*=\s*"([^"]+)"', cargo_text, re.M)
    if not m:
        raise ValueError("no version found in Cargo.toml")
    version = m.group(1)
    if tag.lstrip("v") != version:
        raise ValueError(f"tag {tag} does not match Cargo version {version}")
    return version


def merge(parts):
    """Merge (offset, data) parts into one image, gaps filled with 0xFF."""
    if not parts:
        return b""
    base = min(o for o, _ in parts)
    end = max(o + len(d) for o, d in parts)
    buf = bytearray(FILL * (end - base))
    for o, d in parts:
        buf[o - base:o - base + len(d)] = d
    return bytes(buf)


def _read(path):
    with open(path, "rb") as f:
        return f.read()


def _write(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(data)


def _dump(path, obj):
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        json.dump(obj, f, indent=2)
        f.write("\n")


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--tag", required=True)
    ap.add_argument("--cargo", required=True)
    ap.add_argument("--partitions", required=True)
    ap.add_argument("--bootloader", required=True)
    ap.add_argument("--partition-table", required=True)
    ap.add_argument("--app", required=True)
    ap.add_argument("--page", required=True)
    ap.add_argument("--base-url", default="",
                    help="public site URL (informational; manifests use relative paths)")
    ap.add_argument("--out", required=True)
    a = ap.parse_args(argv)

    try:
        with open(a.cargo, encoding="utf-8") as f:
            version = check_tag(a.tag, f.read())
        with open(a.partitions, encoding="utf-8") as f:
            lay = layout(f.read())
    except ValueError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1

    ota_off, ota_size = lay["otadata"]
    bootloader = _read(a.bootloader)
    ptable = _read(a.partition_table)
    app = _read(a.app)
    otadata = FILL * ota_size

    rel = f"firmware/{version}"
    fw = os.path.join(a.out, "firmware", version)
    app_name = f"airzone-gateway-{version}.bin"
    full_name = f"airzone-gateway-{version}-full.bin"
    parts = [(0x0, bootloader), (0x8000, ptable), (ota_off, otadata), (lay["app"], app)]
    files = {
        "bootloader.bin": bootloader,
        "partition-table.bin": ptable,
        "otadata.bin": otadata,
        app_name: app,
        full_name: merge(parts),
    }
    shutil.rmtree(fw, ignore_errors=True)
    for name, data in files.items():
        _write(os.path.join(fw, name), data)

    manifest = {
        "name": "Open Airzone Gateway",
        "version": version,
        "new_install_prompt_erase": True,
        "builds": [{
            "chipFamily": "ESP32-S3",
            "parts": [
                {"path": f"{rel}/bootloader.bin", "offset": 0x0},
                {"path": f"{rel}/partition-table.bin", "offset": 0x8000},
                {"path": f"{rel}/otadata.bin", "offset": ota_off},
                {"path": f"{rel}/{app_name}", "offset": lay["app"]},
            ],
        }],
    }
    ota = {
        "name": "Open Airzone Gateway",
        "version": version,
        "builds": [{
            "chipFamily": "ESP32-S3",
            "parts": [{"path": f"{rel}/{app_name}", "offset": lay["app"]}],
        }],
    }
    _dump(os.path.join(a.out, "manifest.json"), manifest)
    _dump(os.path.join(a.out, "ota.json"), ota)

    with open(a.page, encoding="utf-8") as f:
        page = f.read().replace("{{VERSION}}", version)
    with open(os.path.join(a.out, "index.html"), "w", encoding="utf-8", newline="\n") as f:
        f.write(page)

    sums = "".join(
        f"{hashlib.sha256(files[n]).hexdigest()}  {rel}/{n}\n" for n in sorted(files)
    )
    with open(os.path.join(a.out, "SHA256SUMS"), "w", encoding="utf-8", newline="\n") as f:
        f.write(sums)
    return 0


if __name__ == "__main__":
    sys.exit(main())
