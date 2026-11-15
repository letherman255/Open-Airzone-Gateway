import os

from generator.schema import load, validate

PATH = os.path.join(os.path.dirname(__file__), "..", "registers.toml")


def params():
    return {p.id: p for p in load(PATH)}


def test_catalogue_is_valid():
    assert validate(load(PATH)) == []


def test_spot_values():
    p = params()
    s = p["zone.setpoint"]
    assert (s.scope, s.reg, s.lo, s.hi, s.scale, s.access, s.confidence) == ("zone", 3, 0, 15, 0.1, "rw", "confirmed")
    assert p["zone.mode"].values == {0: "stop", 3: "fan", 5: "heat", 6: "dry", 9: "cool"}
    d = p["system.d1_input"]
    assert (d.reg, d.lo, d.hi, d.tier, d.danger.level) == (0x0048, 9, 11, 3, "high")
    v = p["iaq.vmc"]
    assert (v.reg, v.lo, v.hi) == (0x2101, 6, 6)
    f = p["system.filter_hours"]
    assert (f.reg, f.min, f.max) == (0x009B, 100, 5000)
    o = p["outlet.cool_stage"]
    assert (o.scope, o.reg, o.lo, o.hi) == ("outlet", 1, 5, 5)


def test_no_writable_hypothesis():
    assert [p.id for p in load(PATH) if p.access == "rw" and p.confidence != "confirmed"] == []


def test_ids_unique_and_count():
    ps = load(PATH)
    assert len({p.id for p in ps}) == len(ps)
    assert len(ps) >= 95


def test_risky_installer_writes_need_confirmation():
    p = params()
    for pid in ("zone.master", "outlet.zone", "outlet.heat_stage", "outlet.cool_stage", "outlet.fancoil",
                "system.centralized_controller"):
        assert p[pid].danger is not None, pid
    assert p["zone.heat_stage_air"].access == "r"
