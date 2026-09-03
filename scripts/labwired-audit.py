#!/usr/bin/env python3
import argparse
import json
import sys
from datetime import datetime
from pathlib import Path


def hex_str(val):
    return f"0x{val:08X}"


def compare_traces(
    hw_trace,
    sim_trace,
    max_steps=None,
    align_window=10,
    allow_prefix=False,
):
    results = []
    scope = {
        "mode": "bounded" if max_steps is not None else "prefix" if allow_prefix else "exact",
        "hw_start_index": None,
        "sim_start_index": None,
        "hw_aligned_steps": 0,
        "sim_aligned_steps": 0,
        "requested_steps": max_steps,
    }

    # Sliding-window search for the first common PC. Hardware traces may carry
    # reset/boot instructions before the simulator's configured entry point.
    hw_start_idx = -1
    sim_start_idx = -1
    found = False
    for s_idx in range(min(len(sim_trace), align_window)):
        sim_pc = sim_trace[s_idx]["pc"]
        for h_idx in range(min(len(hw_trace), align_window * 10)):
            if hw_trace[h_idx]["pc"] == sim_pc:
                sim_start_idx = s_idx
                hw_start_idx = h_idx
                found = True
                break
        if found:
            break

    if not found:
        return "FAIL", "Could not align traces by PC within window.", [], scope

    print(
        f"Aligned: HW step {hw_start_idx} matches SIM step {sim_start_idx} "
        f"(PC: {hex_str(sim_trace[sim_start_idx]['pc'])})"
    )

    hw_aligned = hw_trace[hw_start_idx:]
    sim_aligned = sim_trace[sim_start_idx:]
    scope.update(
        {
            "hw_start_index": hw_start_idx,
            "sim_start_index": sim_start_idx,
            "hw_aligned_steps": len(hw_aligned),
            "sim_aligned_steps": len(sim_aligned),
        }
    )

    if max_steps is not None:
        if max_steps <= 0:
            return "FAIL", "--max-steps must be greater than zero.", [], scope
        if len(hw_aligned) < max_steps or len(sim_aligned) < max_steps:
            return (
                "FAIL",
                f"Bounded comparison requested {max_steps} steps, but aligned traces provide "
                f"HW={len(hw_aligned)} and SIM={len(sim_aligned)}.",
                [],
                scope,
            )
        steps = max_steps
    elif allow_prefix:
        steps = min(len(hw_aligned), len(sim_aligned))
    else:
        if len(hw_aligned) != len(sim_aligned):
            return (
                "FAIL",
                "Aligned trace lengths differ: "
                f"HW={len(hw_aligned)}, SIM={len(sim_aligned)}. "
                "Use --allow-prefix to declare a prefix-only audit or --max-steps N "
                "to declare a bounded audit.",
                [],
                scope,
            )
        steps = len(hw_aligned)

    if steps == 0:
        return "FAIL", "No aligned steps are available for comparison.", [], scope

    matches = 0
    for i in range(steps):
        hw_pc = hw_aligned[i]["pc"]
        sim_pc = sim_aligned[i]["pc"]
        match = hw_pc == sim_pc

        results.append(
            {
                "step": i,
                "hw_pc": hex_str(hw_pc),
                "sim_pc": hex_str(sim_pc),
                "match": match,
            }
        )

        if match:
            matches += 1
        else:
            print(f"Drift at index {i}: HW={hex_str(hw_pc)}, SIM={hex_str(sim_pc)}")
            break

    status = "PASS" if matches == steps else "FAIL"
    notes = f"Verified {matches}/{steps} PC steps match ({scope['mode']} scope)."
    if status == "FAIL":
        notes += f" Drift detected at step {matches}."
    elif allow_prefix and len(hw_aligned) != len(sim_aligned):
        notes += " PASS applies only to the declared common prefix, not the remaining trace."

    return status, notes, results, scope


def main():
    parser = argparse.ArgumentParser(description="LabWired PC trace audit tool")
    parser.add_argument("--hw-trace", required=True, help="Path to hardware trace JSON")
    parser.add_argument("--sim-trace", required=True, help="Path to simulation trace JSON")
    parser.add_argument("--target", default="Unknown", help="Target board/MCU name")
    parser.add_argument("--firmware", default="Unknown", help="Firmware name/version label")
    parser.add_argument("--output", required=True, help="Path to output report JSON")
    scope = parser.add_mutually_exclusive_group()
    scope.add_argument(
        "--max-steps",
        type=int,
        help="Compare exactly N aligned PC steps; both traces must provide N",
    )
    scope.add_argument(
        "--allow-prefix",
        action="store_true",
        help="Compare the complete shorter aligned trace and label the result prefix-only",
    )
    parser.add_argument(
        "--align-window", type=int, default=10, help="Window size for alignment search"
    )

    args = parser.parse_args()

    try:
        hw_trace = json.loads(Path(args.hw_trace).read_text())
        sim_trace = json.loads(Path(args.sim_trace).read_text())
    except Exception as exc:
        print(f"Error loading traces: {exc}")
        sys.exit(1)

    status, notes, steps_results, comparison_scope = compare_traces(
        hw_trace,
        sim_trace,
        args.max_steps,
        args.align_window,
        args.allow_prefix,
    )

    report = {
        "timestamp": datetime.now().strftime("%a %b %d %H:%M:%S %Y"),
        "target": args.target,
        "firmware": args.firmware,
        "verification_tool": "labwired-audit (v0.2.0)",
        "verification_kind": "pc_sequence",
        "comparison_scope": comparison_scope,
        "steps_compared": len(steps_results),
        "status": status,
        "notes": notes,
        "results": steps_results,
        "firmware_integrity": {
            "performed": False,
            "reason": "--firmware is a label; this tool does not receive or hash the firmware binary",
        },
        "checksum_verification": {
            "performed": False,
            "trace_match": "NOT_CHECKED",
        },
    }

    with open(args.output, "w") as handle:
        json.dump(report, handle, indent=2)

    print(f"Audit complete. Status: {status}. Report saved to {args.output}")
    sys.exit(0 if status == "PASS" else 1)


if __name__ == "__main__":
    main()
