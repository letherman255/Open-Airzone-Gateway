"""tools/build_site.py: layout parsing, tag check, merge, full site build."""
import hashlib
import importlib.util
import json
import os
import re
import subprocess
import sys

import pytest

REPO = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
TOOL = os.path.join(REPO, "tools", "build_site.py")


def _load():
    spec = importlib.util.spec_from_file_location("build_site", TOOL)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


@pytest.fixture(scope="module")
def bs():
    return _load()


def test_layout_from_partitions_csv(bs):
    with open(os.path.join(REPO, "firmware", "esp", "partitions.csv")) as f:
        csv = f.read()
    assert bs.layout(csv) == {"otadata": (0xF000, 0x2000), "app": 0x20000}


def test_check_tag(bs):
    assert bs.check_tag("v0.2.0", 'version = "0.2.0"') == "0.2.0"
    with pytest.raises(ValueError):
        bs.check_tag("v0.1.0", 'version = "0.2.0"')


def test_merge_fills_gaps(bs):
    assert bs.merge([(0, b"\x01"), (4, b"\x02")]) == b"\x01\xff\xff\xff\x02"


def _args(tmp_path, tag):
    for n in ("boot", "ptab", "app"):
        (tmp_path / f"{n}.bin").write_bytes(b"\xe9" * 4096)
    (tmp_path / "page.html").write_text("<p>{{VERSION}}</p>")
    # the real Cargo.toml pinned at 0.2.0: the test must not break at each version bump
    with open(os.path.join(REPO, "firmware", "esp", "Cargo.toml"), encoding="utf-8") as f:
        cargo = re.sub(r'(?m)^version = "[^"]*"', 'version = "0.2.0"', f.read(), count=1)
    (tmp_path / "Cargo.toml").write_text(cargo, encoding="utf-8")
    return [
        "--tag", tag,
        "--cargo", str(tmp_path / "Cargo.toml"),
        "--partitions", os.path.join(REPO, "firmware", "esp", "partitions.csv"),
        "--bootloader", str(tmp_path / "boot.bin"),
        "--partition-table", str(tmp_path / "ptab.bin"),
        "--app", str(tmp_path / "app.bin"),
        "--page", str(tmp_path / "page.html"),
        "--base-url", "https://example.github.io/gw",
        "--out", str(tmp_path / "out"),
    ]


def test_site(bs, tmp_path):
    assert bs.main(_args(tmp_path, "v0.2.0")) == 0
    out = tmp_path / "out"
    manifest = json.loads((out / "manifest.json").read_text())
    assert manifest["version"] == "0.2.0" and manifest["new_install_prompt_erase"] is True
    assert [p["offset"] for p in manifest["builds"][0]["parts"]] == [0x0, 0x8000, 0xF000, 0x20000]
    assert manifest["builds"][0]["chipFamily"] == "ESP32-S3"
    assert (out / "firmware/0.2.0/otadata.bin").read_bytes() == b"\xff" * 0x2000
    ota = json.loads((out / "ota.json").read_text())
    assert ota["version"] == "0.2.0"
    assert len(ota["builds"][0]["parts"]) == 1
    path = ota["builds"][0]["parts"][0]["path"]
    assert path.endswith("airzone-gateway-0.2.0.bin") and "://" not in path
    assert not (out / "SHA256SUMS").exists()
    fw = out / "firmware/0.2.0"
    sums = (fw / "SHA256SUMS").read_text()
    assert len(sums.splitlines()) == 5
    # bare names that verify next to the files (as `sha256sum -c` would), like flat release downloads
    for line in sums.splitlines():
        digest, name = line.split("  ")
        assert "/" not in name
        assert hashlib.sha256((fw / name).read_bytes()).hexdigest() == digest
    assert "  airzone-gateway-0.2.0-full.bin\n" in sums
    assert (out / "index.html").read_text() == "<p>0.2.0</p>"
    assert "{{VERSION}}" not in (out / "index.html").read_text()
    full = (out / "firmware/0.2.0/airzone-gateway-0.2.0-full.bin").read_bytes()
    assert len(full) == 0x20000 + 4096 and full[0x20000] == 0xE9


def test_tag_mismatch_exits_1(tmp_path):
    r = subprocess.run([sys.executable, "-I", TOOL] + _args(tmp_path, "v9.9.9"),
                       capture_output=True, text=True)
    assert r.returncode == 1 and "9.9.9" in r.stderr
