"""python -m generator registers.toml --out <repo root>"""
import argparse
import os
import sys

from .emit import emit_all
from .schema import load, validate


def main() -> int:
    ap = argparse.ArgumentParser(prog="generator")
    ap.add_argument("catalogue")
    ap.add_argument("--out", required=True, help="repository root")
    a = ap.parse_args()
    params = load(a.catalogue)
    errors = validate(params)
    if errors:
        for e in errors:
            print(e, file=sys.stderr)
        return 1
    for rel, text in emit_all(params).items():
        path = os.path.join(a.out, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8", newline="\n") as f:
            f.write(text)
        print("wrote", rel)
    return 0


if __name__ == "__main__":
    sys.exit(main())
