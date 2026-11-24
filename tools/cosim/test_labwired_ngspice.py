"""Tests for the ngspice co-simulation wrapper.

Run: python3 -m pytest tools/cosim/test_labwired_ngspice.py -v
Requires libngspice (Debian/Ubuntu: apt install libngspice0).
"""
import io
import json
import math
import os
import subprocess
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


def _bridge_spec(tmp_path, element="Vgpio", node="out", netlist_text=BRIDGE_NETLIST,
                 step_ns=100_000):
    # Core sends the validated text, not a path (see NgspiceModelSpec::host_json).
    return {
        "model_id": "rc",
        "netlist": netlist_text,
        "step_ns": step_ns,
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


def test_bridge_priming_step_is_the_start_of_the_transient(tmp_path):
    # Continuity across the (0, 0) priming step: the primed value is where the
    # transient starts, and the next boundary follows the RC curve from it.
    # Under `uic` that start is the initial condition (C uncharged, 0 V), not
    # the DC operating point (3.3 V), which an `op` would have reported.
    frames = [{"time_ns": 0, "dt_ns": 0, "inputs": {"gpio": True}}] + [
        {"time_ns": k * 100_000, "dt_ns": 100_000, "inputs": {"gpio": True}} for k in (1, 2)]
    rc, lines = _serve(_bridge_spec(tmp_path), frames)
    assert rc == 0 and len(lines) == 3, lines
    assert abs(lines[0]["outputs"]["v_out"]) < 1e-3, lines[0]
    for k, line in ((1, lines[1]), (2, lines[2])):
        expected = 3.3 * (1 - math.exp(-k * 100e-6 / TAU_S))
        assert math.isclose(line["outputs"]["v_out"], expected, abs_tol=3.3 * 2e-3), (k, line)

    # Without `uic` the transient starts from its own operating point, and the
    # primed value is that point (t = 0 is stored) and it stays there.
    no_uic = RC_NETLIST.replace(".end", ".tran 1u 10\n.end")
    rc, lines = _serve(_bridge_spec(tmp_path, netlist_text=no_uic), frames)
    assert rc == 0 and len(lines) == 3, lines
    for line in lines:
        assert math.isclose(line["outputs"]["v_out"], 3.3, rel_tol=1e-6), line


def test_bridge_priming_refuses_a_first_point_past_the_first_boundary(tmp_path):
    # Negative control: a 5 ns boundary period is shorter than ngspice's first
    # internal step under `uic` (10 ns for `.tran 1u`), so the primed state
    # would belong to a later time than the first boundary.
    rc, lines = _serve(_bridge_spec(tmp_path, step_ns=5),
                       [{"time_ns": 0, "dt_ns": 0, "inputs": {"gpio": True}}])
    assert rc == 1 and "first transient point" in lines[0]["error"], lines


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


def test_bridge_passes_every_physical_line_to_ngspice(tmp_path):
    # Blank lines are not dropped, so the title is the line ngspice picks, as
    # in the browser host. Measured on ngspice-42: a blank first line is
    # skipped and the next line is the title.
    assert lw._netlist_lines("\r\n* t\r\n\nV1 a 0 1\n") == ["", "* t", "", "V1 a 0 1"]
    frame = {"time_ns": 100_000, "dt_ns": 100_000, "inputs": {"gpio": True}}
    rc, lines = _serve(_bridge_spec(tmp_path, netlist_text="\n" + BRIDGE_NETLIST), [frame])
    assert rc == 0 and "outputs" in lines[0], lines
    # Negative control: an element on the first line is the title, so it is
    # not in the circuit and the alter fails.
    element_title = BRIDGE_NETLIST.lstrip("\n").split("\n", 1)[1]
    assert element_title.startswith("Vgpio")
    rc, lines = _serve(_bridge_spec(tmp_path, netlist_text=element_title), [frame])
    assert rc == 1 and "vgpio" in lines[0]["error"].lower(), lines


def test_bridge_reads_netlist_path_only_without_netlist_text(tmp_path):
    path = tmp_path / "rc.cir"
    path.write_text(BRIDGE_NETLIST)
    spec = _bridge_spec(tmp_path)
    del spec["netlist"]
    spec["netlist_path"] = str(path)
    frame = {"time_ns": 100_000, "dt_ns": 100_000, "inputs": {"gpio": True}}
    rc, lines = _serve(spec, [frame])
    assert rc == 0 and "outputs" in lines[0], lines
    # The text wins over a path when both are given.
    spec = _bridge_spec(tmp_path)
    spec["netlist_path"] = str(tmp_path / "missing.cir")
    rc, lines = _serve(spec, [frame])
    assert rc == 0 and "outputs" in lines[0], lines


def test_bridge_refuses_control_blocks_and_included_files(tmp_path):
    # An included file could hold a `.control` block with a shell command; the
    # host refuses the directive before ngspice parses anything.
    marker = tmp_path / "shell-ran"
    (tmp_path / "evil.cir").write_text(f"* evil\n.control\nshell touch {marker}\n.endc\n")
    frame = {"time_ns": 100_000, "dt_ns": 100_000, "inputs": {"gpio": True}}
    for directive, name in ((f".include {tmp_path}/evil.cir", ".include"),
                            (f".inc {tmp_path}/evil.cir", ".inc"),
                            (f"  .LIB {tmp_path}/evil.cir tt", ".lib"),
                            (".control", ".control"),
                            ("\t.endc", ".endc")):
        text = BRIDGE_NETLIST.replace(".tran", f"{directive}\n.tran")
        rc, lines = _serve(_bridge_spec(tmp_path, netlist_text=text), [frame])
        assert rc == 1 and f"{name} directive" in lines[0]["error"], (directive, lines)
    assert not marker.exists()


_HOST_SOURCE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "labwired_ngspice.py")


def test_isolated_host_ignores_a_poisoned_working_directory(tmp_path):
    # Core runs `python3 -I -c <source> --bridge-spec <json>`. With -I a
    # json.py or ctypes/ package in the working directory does not load;
    # without it the poison does (negative control).
    poison = tmp_path / "cwd"
    (poison / "ctypes").mkdir(parents=True)
    marker = tmp_path / "poison-ran"
    payload = f"open({str(marker)!r}, 'w').write('ran')\nraise SystemExit('poisoned')\n"
    (poison / "json.py").write_text(payload)
    (poison / "ctypes" / "__init__.py").write_text(payload)
    with open(_HOST_SOURCE, encoding="utf-8") as f:
        source = f.read()
    spec = json.dumps(_bridge_spec(tmp_path))
    frame = json.dumps({"time_ns": 100_000, "dt_ns": 100_000, "inputs": {"gpio": True}}) + "\n"

    def run(*flags):
        return subprocess.run([sys.executable, *flags, "-c", source, "--bridge-spec", spec],
                              cwd=poison, input=frame, capture_output=True, text=True,
                              timeout=60)

    isolated = run("-I")
    assert isolated.returncode == 0, isolated.stderr
    assert "outputs" in json.loads(isolated.stdout.splitlines()[0])
    assert not marker.exists()
    exposed = run()
    assert exposed.returncode != 0 and marker.exists()
