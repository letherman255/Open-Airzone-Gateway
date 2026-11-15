"""Load and validate catalogue/registers.toml (the single source of truth for every register)."""
from __future__ import annotations

import tomllib
from dataclasses import dataclass, field

SCOPES = ("system", "zone", "iaq", "outlet")
KINDS = ("number", "enum", "bool", "text", "command")
CONFIDENCES = ("confirmed", "matched", "hypothesis")
REQUIRES = ("iaq_present", "vmc_on", "ionizer")


@dataclass
class Danger:
    level: str
    warning: str = ""
    confirm: str = ""


@dataclass
class Param:
    id: str
    scope: str
    reg: int
    lo: int
    hi: int
    kind: str
    access: str
    write: str
    tier: int
    confidence: str
    evidence: str = ""
    scale: float = 1.0
    offset: float = 0.0
    unit: str = ""
    values: dict[int, str] = field(default_factory=dict)
    min: float | None = None
    max: float | None = None
    length: int = 1
    requires: str | None = None
    danger: Danger | None = None
    ha: dict | None = None
    ui: dict = field(default_factory=dict)


def _bits(text: str) -> tuple[int, int]:
    lo, _, hi = text.partition("-")
    return int(lo), int(hi or lo)


def load(path: str) -> list[Param]:
    with open(path, "rb") as f:
        data = tomllib.load(f)
    out = []
    for e in data.get("param", []):
        lo, hi = _bits(e.get("bits", "0-15"))
        danger = e.get("danger")
        out.append(Param(
            id=e["id"], scope=e["scope"], reg=int(e["reg"]), lo=lo, hi=hi, kind=e["kind"],
            access=e.get("access", "r"),
            write=e.get("write", "06" if (lo, hi) == (0, 15) else "16"),
            tier=int(e["tier"]), confidence=e["confidence"], evidence=e.get("evidence", ""),
            scale=float(e.get("scale", 1.0)), offset=float(e.get("offset", 0.0)), unit=e.get("unit", ""),
            values={int(k): v for k, v in e.get("values", {}).items()},
            min=e.get("min"), max=e.get("max"), length=int(e.get("length", 1)),
            requires=e.get("requires"),
            danger=Danger(**danger) if danger else None,
            ha=e.get("ha"), ui=e.get("ui", {}),
        ))
    return out


def validate(params: list[Param]) -> list[str]:
    errors: list[str] = []
    seen: set[str] = set()
    for p in params:
        if p.id in seen:
            errors.append(f"{p.id}: duplicate id")
            continue
        seen.add(p.id)
        if p.scope not in SCOPES:
            errors.append(f"{p.id}: unknown scope '{p.scope}'")
        if p.kind not in KINDS:
            errors.append(f"{p.id}: unknown kind '{p.kind}'")
        if p.confidence not in CONFIDENCES:
            errors.append(f"{p.id}: unknown confidence '{p.confidence}'")
        if not (0 <= p.lo <= p.hi <= 15):
            errors.append(f"{p.id}: bad bits {p.lo}-{p.hi}")
        if p.requires is not None and p.requires not in REQUIRES:
            errors.append(f"{p.id}: unknown requires '{p.requires}'")
        if p.danger is not None and not (p.danger.warning and p.danger.confirm):
            errors.append(f"{p.id}: danger needs warning and confirm")
        if p.tier == 3 and p.ha:
            errors.append(f"{p.id}: tier 3 must not have ha")
        if p.access == "rw":
            if not p.evidence:
                errors.append(f"{p.id}: rw needs evidence")
            if p.confidence != "confirmed":
                errors.append(f"{p.id}: rw needs confidence confirmed")
        if p.kind == "enum" and not p.values:
            errors.append(f"{p.id}: enum needs values")
    first = {}
    for p in params:
        first.setdefault(p.id, p)
    unique = list(first.values())
    for i, a in enumerate(unique):
        for b in unique[i + 1:]:
            if a.scope != b.scope:
                continue
            for k in range(max(a.length, 1)):
                for m in range(max(b.length, 1)):
                    if a.reg + k != b.reg + m:
                        continue
                    lo, hi = max(a.lo, b.lo), min(a.hi, b.hi)
                    if lo <= hi:
                        errors.append(f"{a.id} and {b.id} overlap in {a.scope} reg 0x{a.reg + k:04X} bits {lo}-{hi}")
    return errors
