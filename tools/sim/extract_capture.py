"""Copy the lines of a bus capture between two timestamps.

usage: python -I extract_capture.py <capture log> <from> <to> <out>
Timestamps are ISO prefixes compared as text, e.g. 2026-11-05T21:13:45 2026-11-05T21:15:10.
Capture line format: <iso ts> <mono> <len> <hex bytes…>.
"""
import sys


def main():
    src, start, end, out = sys.argv[1:5]
    n = 0
    with open(src, encoding="utf-8", errors="replace") as f, open(out, "w", encoding="utf-8", newline="\n") as o:
        for line in f:
            ts = line[:len(start)]
            if start <= ts <= end and len(line.split()) > 3:
                o.write(line.rstrip("\n") + "\n")
                n += 1
    print(n, "lines")


if __name__ == "__main__":
    main()
