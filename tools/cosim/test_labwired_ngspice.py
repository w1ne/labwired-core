"""Tests for the ngspice co-simulation wrapper.

Run: python3 -m pytest tools/cosim/test_labwired_ngspice.py -v
Requires libngspice (Debian/Ubuntu: apt install libngspice0).
"""
import io
import json
import math
import os
import sys
import textwrap

sys.path.insert(0, os.path.dirname(__file__))

import labwired_ngspice as lw  # noqa: E402

RC_NETLIST = textwrap.dedent(
    """
    * GPIO drives an RC low-pass; firmware ADC reads node out
    Vgpio in 0 dc 0
    R1 in out 10k
    C1 out 0 100n
    .end
    """
)
TAU_S = 10e3 * 100e-9  # 1 ms


def test_rc_charges_to_63_percent_after_one_tau():
    sim = lw.NgSpice(RC_NETLIST, sources={"gpio": "Vgpio"}, probes={"v_out": "v(out)"})
    sim.set_source("gpio", 3.3)
    out = sim.step_to(TAU_S)
    assert math.isclose(out["v_out"], 3.3 * (1 - math.exp(-1)), rel_tol=0.02)


def test_lockstep_steps_are_sequential_and_deterministic():
    def run():
        sim = lw.NgSpice(RC_NETLIST, sources={"gpio": "Vgpio"}, probes={"v_out": "v(out)"})
        trace = []
        for i in range(1, 6):
            sim.set_source("gpio", 3.3 if i <= 3 else 0.0)
            trace.append(round(sim.step_to(i * 0.5e-3)["v_out"], 6))
        return trace

    a, b = run(), run()
    assert a == b
    assert a[0] < a[1] < a[2]          # charging
    assert a[3] < a[2] and a[4] < a[3]  # discharging after gpio low


def test_jsonl_serve_maps_bool_inputs_to_vdd_and_returns_probes():
    stdin = io.StringIO(
        # time_ns is the END of each step, as CosimRunner sends it
        json.dumps({"time_ns": 1_000_000, "dt_ns": 1_000_000, "inputs": {"gpio": True}}) + "\n"
        + json.dumps({"time_ns": 2_000_000, "dt_ns": 1_000_000, "inputs": {"gpio": False}}) + "\n"
    )
    stdout = io.StringIO()
    lw.serve(
        RC_NETLIST,
        sources={"gpio": "Vgpio"},
        probes={"v_out": "v(out)"},
        vdd=3.3,
        stdin=stdin,
        stdout=stdout,
    )
    lines = [json.loads(l) for l in stdout.getvalue().splitlines()]
    assert len(lines) == 2
    v1, v2 = lines[0]["outputs"]["v_out"], lines[1]["outputs"]["v_out"]
    assert math.isclose(v1, 3.3 * (1 - math.exp(-1)), rel_tol=0.02)
    assert v2 < v1


def test_missing_source_name_is_a_clear_error():
    sim = lw.NgSpice(RC_NETLIST, sources={"gpio": "Vgpio"}, probes={"v_out": "v(out)"})
    try:
        sim.set_source("nope", 1.0)
    except KeyError as e:
        assert "nope" in str(e)
    else:
        raise AssertionError("expected KeyError")
