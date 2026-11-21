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


# --- adapter: ngspice host (`--bridge-spec`) --------------------------------

BRIDGE_NETLIST = RC_NETLIST.replace(".end", ".tran 1u 10 uic\n.end")


def _bridge_spec(tmp_path, element="Vgpio", node="out"):
    netlist = tmp_path / "rc.cir"
    netlist.write_text(BRIDGE_NETLIST)
    return {
        "model_id": "rc",
        "netlist_path": str(netlist),
        "inputs": {"gpio": {"element": element, "scale": 3.3, "offset": 0.0,
                            "default_value": None, "logic": True}},
        "outputs": {"v_out": {"node": node, "scale": 1.0, "offset": 0.0, "threshold": None},
                    "high": {"node": node, "scale": 1.0, "offset": 0.0, "threshold": 1.65}},
    }


def _serve(spec, frames):
    stdout = io.StringIO()
    rc = lw.serve_bridge(spec, io.StringIO("".join(json.dumps(f) + "\n" for f in frames)), stdout)
    return rc, [json.loads(line) for line in stdout.getvalue().splitlines()]


def test_bridge_steps_land_on_exact_boundaries(tmp_path):
    # 200 us is the boundary ngspice-42 overshot to the .tran end without a
    # breakpoint; the host refuses any step that does not end on time_ns.
    frames = [{"time_ns": k * 100_000, "dt_ns": 100_000, "inputs": {"gpio": True}}
              for k in range(1, 31)]
    rc, lines = _serve(_bridge_spec(tmp_path), frames)
    assert rc == 0 and len(lines) == 30
    for k, line in enumerate(lines, start=1):
        expected = 3.3 * (1 - math.exp(-k * 100e-6 / TAU_S))
        assert math.isclose(line["outputs"]["v_out"], expected, abs_tol=3.3 * 2e-3), (k, line)
    assert lines[0]["outputs"]["high"] is False and lines[-1]["outputs"]["high"] is True


def test_bridge_priming_step_reports_the_operating_point(tmp_path):
    rc, lines = _serve(_bridge_spec(tmp_path), [
        {"time_ns": 0, "dt_ns": 0, "inputs": {"gpio": True}},
        {"time_ns": 100_000, "dt_ns": 100_000, "inputs": {"gpio": True}},
    ])
    assert rc == 0
    assert math.isclose(lines[0]["outputs"]["v_out"], 3.3, rel_tol=1e-9)  # DC: C open
    assert lines[1]["outputs"]["v_out"] < 1.0  # .tran uic restarts from 0 V


def test_bridge_errors_are_one_frame_then_exit(tmp_path):
    frame = {"time_ns": 1000, "dt_ns": 1000, "inputs": {"gpio": True}}
    rc, lines = _serve(_bridge_spec(tmp_path, element="Vnope"), [frame, frame])
    assert rc == 1 and len(lines) == 1 and "vnope" in lines[0]["error"]
    rc, lines = _serve(_bridge_spec(tmp_path, node="nowhere"), [frame])
    assert rc == 1 and "nowhere" in lines[0]["error"]
    rc, lines = _serve(_bridge_spec(tmp_path), [frame, frame])
    assert rc == 1 and "outputs" in lines[0] and "monotonically" in lines[1]["error"]
    rc, lines = _serve(_bridge_spec(tmp_path), [{**frame, "inputs": {"gpio": 1.0}}])
    assert rc == 1 and "must be boolean" in lines[0]["error"]
