#!/usr/bin/env python3
"""Independent timing oracle over real emulator GPIO and serial artifacts.

This reads LabWired's output. It does NOT model the firmware or plant.
"""
import json
import sys
from pathlib import Path

CYCLES_PER_MS = 8000
EXPECTED = {
    "stale": [1, 0],
    "missing": [1, 0],
    "malformed": [1, 0],
    "healthy": [1],
    "no-start": [],
    "recovery": [1, 0, 1],
}

def require(condition, reason):
    if not condition:
        raise AssertionError(reason)

def verify(case, filename):
    data = json.loads(Path(filename).read_text())
    require(data.get("status") == "pass", "runner did not pass")
    require(data.get("stop_reason") == "max_cycles", "unexpected stop reason")
    require(all(a.get("passed") for a in data.get("assertions", [])), "runner assertions failed")
    gaps = {g["kind"] for g in data.get("fidelity", [])}
    require(gaps <= {"unpowered_rail_assumed"}, f"unexpected model gaps: {gaps}")
    logic = data.get("logic_edges") or {}
    require(logic.get("dropped") == 0, "GPIO edge buffer overflow")
    channels = logic.get("channels") or []
    require(len(channels) == 1, "missing or ambiguous motor-output trace")
    ch = channels[0]
    require((ch.get("peripheral"), ch.get("pin")) == ("gpiob", 0), "wrong pin")
    require(ch.get("initial") == 0, "MOTOR_ENABLE high before START")
    require(not ch.get("gaps"), "incomplete GPIO trace")
    transitions = ch.get("transitions", [])
    require([int(t["value"]) for t in transitions] == EXPECTED[case],
            f"unexpected PB0 transitions: {transitions}")
    if case != "no-start":
        first_high = transitions[0]["cycle"]
        require(50*CYCLES_PER_MS <= first_high <= 55*CYCLES_PER_MS, "START missed deadline")
    if case in ("stale", "missing", "malformed", "recovery"):
        low = transitions[1]["cycle"]
        require(100*CYCLES_PER_MS < low <= 205*CYCLES_PER_MS,
                f"motor disabled at {low/CYCLES_PER_MS:.3f}ms, later than 205ms")
    if case == "recovery":
        resumed = transitions[2]["cycle"]
        require(400*CYCLES_PER_MS <= resumed <= 405*CYCLES_PER_MS,
                "reset recovery resumed before explicit START or too late")
    pin = (data.get("gpio_pins") or [None])[0]
    final = case in ("healthy", "recovery")
    require(pin and pin.get("mode") == "output", "PB0 not an active output")
    require(pin.get("pad") is final and pin.get("output") is final,
            "incorrect final motor-output pad level")
    console = [line["text"] for line in data.get("uart_lines", {}).get("lines", [])]
    require(console.count("FRESH seq=6 ms=100") == 1, "last fresh frame not accepted at 100ms")
    if case in ("stale", "missing", "malformed", "recovery"):
        require("FAULT_LATCHED ms=200 state=3 PB0=0" in console,
                "timeout event not observed at 200ms")
    if case == "no-start":
        require(not any(s.startswith(("RUNNING", "FAULT_LATCHED")) for s in console),
                "motors ran without START")
    if case == "healthy":
        require(not any(s.startswith("FAULT_LATCHED") for s in console),
                "false sensor timeout during healthy run")
    if case == "recovery":
        require(sum(s.startswith("RUNNING") for s in console) == 2,
                "manual reset + three fresh frames + new START not demonstrated")
    observation_ms = data["cycles"] / CYCLES_PER_MS
    require(observation_ms >= (475 if case == "recovery" else 300),
            "simulation ended before observation window")
    print(f"PASS {case}: PB0 edges "
          f"{[(round(t['cycle']/CYCLES_PER_MS, 3), int(t['value'])) for t in transitions]}"
          f"; observed through {observation_ms:.1f}ms"
          f"; supply={'ideal 3.3V assumed' if gaps else 'modeled'}")

if __name__ == "__main__":
    if len(sys.argv) != 3 or sys.argv[1] not in EXPECTED:
        sys.exit("usage: verify_waveform.py CASE result.json")
    try:
        verify(sys.argv[1], sys.argv[2])
    except (AssertionError, KeyError, TypeError, ValueError) as error:
        sys.exit(f"FAIL {sys.argv[1]}: {error}")
