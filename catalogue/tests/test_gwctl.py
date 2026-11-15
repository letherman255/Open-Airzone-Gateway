"""gwctl against the host dev server (simulated central, master mode)."""
import json
import os
import socket
import subprocess
import sys
import time
import urllib.request

import pytest

REPO = os.path.join(os.path.dirname(__file__), "..", "..")
EXE = os.path.join(REPO, "target", "debug", "devserver" + (".exe" if os.name == "nt" else ""))
GWCTL = os.path.join(REPO, "tools", "gwctl.py")


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def start(*extra):
    if not os.path.exists(EXE):
        pytest.skip("build the dev server first: cargo build -p airzone-sim --bin devserver")
    port = free_port()
    proc = subprocess.Popen([EXE, str(port), *extra], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    host = f"127.0.0.1:{port}"
    for _ in range(50):
        try:
            urllib.request.urlopen(f"http://{host}/api/v1/info", timeout=1)
            break
        except OSError:
            time.sleep(0.1)
    return proc, host


@pytest.fixture(scope="module")
def server():
    proc, host = start()
    def post(path, body):
        req = urllib.request.Request(f"http://{host}/api/v1/{path}", data=json.dumps(body).encode(), method="POST")
        req.add_header("Content-Type", "application/json")
        return json.loads(urllib.request.urlopen(req).read())
    post("password", {"new": "gwctl-test-pw"})
    token = post("login", {"password": "gwctl-test-pw"})["token"]
    yield host, token
    proc.kill()


def run(host, *args, token=None):
    cmd = [sys.executable, "-I", GWCTL, "--host", host] + (["--token", token] if token else []) + list(args)
    return subprocess.run(cmd, capture_output=True, text=True, timeout=60)


def test_gwctl_info(server):
    host, _ = server
    r = run(host, "info")
    assert r.returncode == 0, r.stderr
    assert json.loads(r.stdout)["mode"] == "master"


def test_gwctl_set_and_wait(server):
    host, token = server
    r = run(host, "set", "zone.setpoint", "22.5", "--zone", "3", token=token)
    assert r.returncode == 0, r.stderr
    assert json.loads(r.stdout)["state"] == "applied"
    r = run(host, "get", "zone.setpoint", "--zone", "3")
    assert json.loads(r.stdout)["value"] == 22.5


def test_gwctl_error_exit_codes(server):
    host, token = server
    r = run(host, "set", "zone.setpoint", "21", "--zone", "2")
    assert r.returncode == 1 and "401" in r.stderr
    r = run(host, "get", "zone.nothing", "--zone", "2")
    assert r.returncode == 1 and "404" in r.stderr
    r = run(host, "set", "system.d1_input", "bridge alarm", token=token)
    assert r.returncode == 1 and "403" in r.stderr
    r = run("127.0.0.1:1", "info")
    assert r.returncode == 1 and "cannot reach" in r.stderr


def test_gwctl_watch_and_backup(server, tmp_path):
    host, token = server
    r = run(host, "watch", "--seconds", "3")
    assert r.returncode == 0, r.stderr
    assert '"topic":"values"' in r.stdout
    f = tmp_path / "b.json"
    r = run(host, "backup", str(f), token=token)
    assert r.returncode == 0, r.stderr
    assert json.load(open(f))["format"] == "airzone-gateway-backup"


def test_gwctl_restore_reports_skips(server, tmp_path):
    host, token = server
    f = tmp_path / "r.json"
    f.write_text(json.dumps({"format": "airzone-gateway-backup", "version": 1,
                             "entries": [{"id": "zone.antifreeze", "system": 1, "zone": 9, "value": True}]}))
    r = run(host, "restore", str(f), token=token)
    assert r.returncode == 1, r.stdout + r.stderr


def test_gwctl_update_reports_state(server):
    host, _ = server
    r = run(host, "update")
    assert r.returncode == 0, r.stderr
    # the dev server has no update manifest: checks are off, not "up to date"
    assert "update check off" in r.stdout
    assert "up to date" not in r.stdout
    assert "update_state: idle" in r.stdout


def test_gwctl_update_shows_offer_and_last_check():
    proc, host = start("--fake-update", "9.9.9")
    try:
        r = run(host, "update")
        assert r.returncode == 0, r.stderr
        assert "latest_version: 9.9.9" in r.stdout
        assert "last check: uptime 0 s ok" in r.stdout
    finally:
        proc.kill()
