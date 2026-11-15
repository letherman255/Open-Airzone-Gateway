"""gwctl: command line client for the gateway API (Python standard library only).

usage:
  gwctl [--host HOST] [--token T | --token-file F] COMMAND ...
  info                                   gateway facts
  login PASSWORD                         print a session token
  get PARAM [--system N] [--zone N] [--outlet N]
  set PARAM VALUE [--system N] [--zone N] [--outlet N] [--confirm WORD]   (waits for the result)
  raw ADDR REG [--count N]               raw registers (developer mode)
  watch [--seconds S]                    print WebSocket messages
  backup FILE | restore FILE [--confirm W1,W2]
  ota IMAGE                              upload and confirm a firmware image
Token: --token, --token-file (a file with the token), or the GWCTL_TOKEN environment variable.
Exit status 0 on success, 1 on an API error (the error is printed on stderr).
"""
import argparse
import base64
import json
import os
import socket
import sys
import time
import urllib.error
import urllib.request


class ApiError(Exception):
    pass


class Client:
    def __init__(self, host, token=None):
        self.host = host
        self.token = token

    def call(self, method, path, body=None, raw=None, content_type="application/json", timeout=30):
        data = raw if raw is not None else (json.dumps(body).encode() if body is not None else b"" if method != "GET" else None)
        req = urllib.request.Request(f"http://{self.host}/api/v1/{path}", data=data, method=method)
        req.add_header("Content-Type", content_type)
        if self.token:
            req.add_header("Authorization", "Bearer " + self.token)
        try:
            with urllib.request.urlopen(req, timeout=timeout) as r:
                return json.loads(r.read() or b"null")
        except urllib.error.HTTPError as e:
            try:
                msg = json.loads(e.read()).get("error") or str(e)
            except Exception:
                msg = str(e)
            raise ApiError(f"HTTP {e.code}: {msg}")
        except urllib.error.URLError as e:
            raise ApiError(f"cannot reach {self.host}: {e.reason}")

    def wait_write(self, wid, timeout=30):
        end = time.time() + timeout
        while time.time() < end:
            r = self.call("GET", f"writes/{wid}")
            if r.get("state") != "queued":
                return r
            time.sleep(0.5)
        raise ApiError(f"write {wid} still queued after {timeout} s")

    def watch(self, seconds):
        host, _, port = self.host.partition(":")
        s = socket.create_connection((host, int(port or 80)), timeout=10)
        key = base64.b64encode(os.urandom(16)).decode()
        s.sendall((f"GET /api/v1/ws HTTP/1.1\r\nHost: {self.host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
                   f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n").encode())
        buf = b""
        while b"\r\n\r\n" not in buf:
            chunk = s.recv(4096)
            if not chunk:
                raise ApiError("websocket refused")
            buf += chunk
        head, buf = buf.split(b"\r\n\r\n", 1)
        if b" 101 " not in head.split(b"\r\n")[0]:
            raise ApiError("websocket refused: " + head.split(b"\r\n")[0].decode())
        end = time.time() + seconds
        s.settimeout(1)
        while time.time() < end:
            try:
                chunk = s.recv(65536)
                if not chunk:
                    break
                buf += chunk
            except socket.timeout:
                continue
            while len(buf) >= 2:
                n = buf[1] & 0x7F
                i = 2
                if n == 126:
                    if len(buf) < 4:
                        break
                    n, i = int.from_bytes(buf[2:4], "big"), 4
                elif n == 127:
                    if len(buf) < 10:
                        break
                    n, i = int.from_bytes(buf[2:10], "big"), 10
                if len(buf) < i + n:
                    break
                yield buf[i:i + n].decode(errors="replace")
                buf = buf[i + n:]
        s.close()


def target(a):
    t = {"system": a.system}
    if a.zone is not None:
        t["zone"] = a.zone
    if a.outlet is not None:
        t["outlet"] = a.outlet
    return t


def value_of(text):
    for conv in (int, float):
        try:
            return conv(text)
        except ValueError:
            pass
    return {"true": True, "false": False, "on": True, "off": False}.get(text.lower(), text)


def main(argv=None):
    ap = argparse.ArgumentParser(prog="gwctl", description="open-airzone-gateway API client")
    ap.add_argument("--host", default=os.environ.get("GWCTL_HOST", "airzone-gw.local"))
    ap.add_argument("--token")
    ap.add_argument("--token-file")
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("info")
    p = sub.add_parser("login")
    p.add_argument("password")
    for name in ("get", "set"):
        p = sub.add_parser(name)
        p.add_argument("param")
        if name == "set":
            p.add_argument("value")
            p.add_argument("--confirm")
        p.add_argument("--system", type=int, default=1)
        p.add_argument("--zone", type=int)
        p.add_argument("--outlet", type=int)
    p = sub.add_parser("raw")
    p.add_argument("addr", type=int)
    p.add_argument("reg")
    p.add_argument("--count", type=int, default=1)
    p = sub.add_parser("watch")
    p.add_argument("--seconds", type=float, default=10)
    p = sub.add_parser("backup")
    p.add_argument("file")
    p = sub.add_parser("restore")
    p.add_argument("file")
    p.add_argument("--confirm", default="")
    p = sub.add_parser("ota")
    p.add_argument("image")
    a = ap.parse_args(argv)
    token = a.token or os.environ.get("GWCTL_TOKEN")
    if a.token_file:
        token = open(a.token_file, encoding="utf-8").read().strip()
    c = Client(a.host, token)
    try:
        if a.cmd == "info":
            print(json.dumps(c.call("GET", "info"), indent=1))
        elif a.cmd == "login":
            print(c.call("POST", "login", {"password": a.password})["token"])
        elif a.cmd == "get":
            q = "&".join(f"{k}={v}" for k, v in target(a).items())
            print(json.dumps(c.call("GET", f"params/{a.param}?{q}")))
        elif a.cmd == "set":
            body = dict(target(a), value=value_of(a.value))
            if a.confirm:
                body["confirm"] = a.confirm
            wid = c.call("POST", f"params/{a.param}", body)["queued"]
            r = c.wait_write(wid)
            print(json.dumps(r))
            if r.get("state") != "applied":
                return 1
        elif a.cmd == "raw":
            print(json.dumps(c.call("GET", f"raw/{a.addr}/{a.reg}?count={a.count}")))
        elif a.cmd == "watch":
            for m in c.watch(a.seconds):
                print(m, flush=True)
        elif a.cmd == "backup":
            data = c.call("GET", "backup")
            with open(a.file, "w", encoding="utf-8") as f:
                json.dump(data, f, indent=1)
            print(f"{len(data['entries'])} settings saved to {a.file}")
        elif a.cmd == "restore":
            body = json.load(open(a.file, encoding="utf-8"))
            body["confirm"] = [w.strip() for w in a.confirm.split(",") if w.strip()]
            r = c.call("POST", "restore", body)
            print(json.dumps(r, indent=1))
            failed = len(r["skipped"])
            for wid in r["queued"]:
                res = c.wait_write(wid)
                print(json.dumps(res))
                failed += res.get("state") != "applied"
            if failed:
                print(f"error: {failed} setting(s) not restored", file=sys.stderr)
                return 1
        elif a.cmd == "ota":
            img = open(a.image, "rb").read()
            if not img or img[0] != 0xE9:
                raise ApiError("not an ESP app image")
            print(json.dumps(c.call("POST", "ota", raw=img, content_type="application/octet-stream", timeout=300)))
            print("waiting for the gateway to restart…")
            for _ in range(60):
                time.sleep(3)
                try:
                    print(json.dumps(c.call("POST", "ota/confirm", {})))
                    break
                except ApiError:
                    continue
            else:
                raise ApiError("gateway did not come back: the update will roll back")
    except ApiError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
