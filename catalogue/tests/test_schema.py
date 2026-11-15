import textwrap

from generator.schema import load, validate

BASE = '''
[[param]]
id = "{id}"
scope = "{scope}"
reg = {reg}
bits = "{bits}"
kind = "{kind}"
access = "{access}"
tier = {tier}
confidence = "{confidence}"
evidence = "{evidence}"
{extra}
'''


def entry(id="zone.x", scope="zone", reg=3, bits="0-15", kind="number", access="rw", tier=1,
          confidence="confirmed", evidence="test 2026-11-06", extra=""):
    return BASE.format(id=id, scope=scope, reg=reg, bits=bits, kind=kind, access=access, tier=tier,
                       confidence=confidence, evidence=evidence, extra=textwrap.dedent(extra))


def check(tmp_path, *entries):
    p = tmp_path / "c.toml"
    p.write_text("".join(entries), encoding="utf-8")
    return validate(load(str(p)))


def test_valid_minimal(tmp_path):
    assert check(tmp_path, entry()) == []


def test_overlapping_bits(tmp_path):
    assert check(tmp_path, entry(id="zone.a", bits="0-3"), entry(id="zone.b", bits="2-5")) == [
        "zone.a and zone.b overlap in zone reg 0x0003 bits 2-3"]


def test_danger_needs_warning_and_confirm(tmp_path):
    e = entry(id="system.d1_input", scope="system", reg=0x48, bits="9-11", kind="enum", tier=3,
              extra='values = {0 = "off"}\ndanger = { level = "high" }')
    assert check(tmp_path, e) == ["system.d1_input: danger needs warning and confirm"]


def test_tier3_has_no_ha(tmp_path):
    e = entry(id="system.x", scope="system", reg=7, tier=3, extra='ha = { component = "switch" }')
    assert check(tmp_path, e) == ["system.x: tier 3 must not have ha"]


def test_rw_needs_evidence(tmp_path):
    assert check(tmp_path, entry(evidence="")) == ["zone.x: rw needs evidence"]


def test_rw_needs_confirmed(tmp_path):
    assert check(tmp_path, entry(confidence="matched")) == ["zone.x: rw needs confidence confirmed"]


def test_unknown_requires(tmp_path):
    e = entry(id="iaq.x", scope="iaq", reg=0x2101, extra='requires = "foo"')
    assert check(tmp_path, e) == ["iaq.x: unknown requires 'foo'"]


def test_duplicate_id(tmp_path):
    assert check(tmp_path, entry(bits="0-3"), entry(reg=4)) == ["zone.x: duplicate id"]


def test_write_default(tmp_path):
    p = tmp_path / "c.toml"
    p.write_text(entry(id="zone.a") + entry(id="zone.b", reg=0, bits="2-2", kind="bool"), encoding="utf-8")
    a, b = sorted(load(str(p)), key=lambda x: x.id)
    assert (a.write, a.lo, a.hi) == ("06", 0, 15)
    assert (b.write, b.lo, b.hi) == ("16", 2, 2)
