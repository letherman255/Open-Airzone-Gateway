"""Send a firmware image to the gateway over Wi-Fi (TCP 7001) and confirm it.

usage: python -I tools/ota_upload.py <ip> <image.bin> --token-file <env file with OTA_TOKEN=…>
       python -I tools/ota_upload.py <ip> --probe --token-file …

After the upload the board reboots on the new image, which stays "pending verify". This tool then
waits for the board and sends an authenticated probe (`AZOTA1 <token> 0`): the new image confirms
itself only when that probe succeeds, which proves its own OTA path works. Without the probe the
board resets after 180 s and the bootloader rolls back to the previous image.
"""
import argparse
import socket
import sys
import time

PORT = 7001


def read_token(path):
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if line.startswith("OTA_TOKEN="):
                return line.split("=", 1)[1].strip()
    sys.exit(f"OTA_TOKEN not found in {path}")


def read_line(sock):
    data = b""
    while not data.endswith(b"\n"):
        c = sock.recv(1)
        if not c:
            break
        data += c
    return data.decode(errors="replace").strip()


def upload(ip, token, img):
    with socket.create_connection((ip, PORT), timeout=60) as s:
        s.sendall(f"AZOTA1 {token} {len(img)}\n".encode())
        r = read_line(s)
        if r != "OK ready":
            sys.exit(f"refused: {r}")
        step = 16384
        for i in range(0, len(img), step):
            s.sendall(img[i:i + step])
            print(f"\r  {min(i + step, len(img)) * 100 // len(img):3d}%", end="", flush=True)
        print()
        r = read_line(s)
        print(r)
        if r != "OK done, rebooting":
            sys.exit(f"upload failed: {r}")


def probe(ip, token, wait_s=150):
    """Returns the build id the board reports (`OK probe build=<id>`), or None."""
    deadline = time.time() + wait_s
    while time.time() < deadline:
        try:
            with socket.create_connection((ip, PORT), timeout=5) as s:
                s.sendall(f"AZOTA1 {token} 0\n".encode())
                r = read_line(s)
                print("probe:", r)
                return r.split("build=", 1)[1] if r.startswith("OK probe build=") else None
        except OSError:
            time.sleep(3)
    print("probe: board did not come back")
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("ip")
    ap.add_argument("image", nargs="?")
    ap.add_argument("--token-file", required=True)
    ap.add_argument("--probe", action="store_true", help="only send the confirmation probe")
    a = ap.parse_args()
    token = read_token(a.token_file)
    if a.probe:
        sys.exit(0 if probe(a.ip, token, wait_s=10) else 1)
    with open(a.image, "rb") as f:
        img = f.read()
    if not img or img[0] != 0xE9:
        sys.exit("not an ESP app image (first byte must be 0xE9): use espflash save-image")
    before = probe(a.ip, token, wait_s=10)
    print(f"running build before: {before}; uploading {len(img)} bytes to {a.ip}:{PORT}")
    upload(a.ip, token, img)
    time.sleep(8)
    after = probe(a.ip, token)
    if after is None or after == before:
        sys.exit(f"new image not confirmed (build before {before}, after {after}): it will roll back")
    print(f"confirmed build {after}")


if __name__ == "__main__":
    main()
