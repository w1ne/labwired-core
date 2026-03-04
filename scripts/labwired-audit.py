#!/usr/bin/env python3
import argparse
import json
import sys
from datetime import datetime
from pathlib import Path

def hex_str(val):
    return f"0x{val:08X}"

def compare_traces(hw_trace, sim_trace, max_steps=None, align_window=10):
    results = []
    
    # Improved alignment: Sliding window search for first common PC
    hw_start_idx = -1
    sim_start_idx = -1
    
    # Try finding any PC from early SIM trace in HW trace
    found = False
    for s_idx in range(min(len(sim_trace), align_window)):
        sim_pc = sim_trace[s_idx]["pc"]
        for h_idx in range(min(len(hw_trace), align_window * 10)): # HW trace might have more junk
            if hw_trace[h_idx]["pc"] == sim_pc:
                sim_start_idx = s_idx
                hw_start_idx = h_idx
                found = True
                break
        if found: break
        
    if not found:
        return "FAIL", "Could not align traces by PC within window.", []

    print(f"Aligned: HW step {hw_start_idx} matches SIM step {sim_start_idx} (PC: {hex_str(sim_trace[sim_start_idx]['pc'])})")
    
    hw_aligned = hw_trace[hw_start_idx:]
    sim_aligned = sim_trace[sim_start_idx:]
    
    steps = min(len(hw_aligned), len(sim_aligned))
    if max_steps:
        steps = min(steps, max_steps)
        
    matches = 0
    for i in range(steps):
        hw_pc = hw_aligned[i]["pc"]
        sim_pc = sim_aligned[i]["pc"]
        match = (hw_pc == sim_pc)
        
        results.append({
            "step": i,
            "hw_pc": hex_str(hw_pc),
            "sim_pc": hex_str(sim_pc),
            "match": match
        })
        
        if match:
            matches += 1
        else:
            print(f"Drift at index {i}: HW={hex_str(hw_pc)}, SIM={hex_str(sim_pc)}")
            break # Stop on first drift for now
            
    status = "PASS" if matches == steps else "FAIL"
    notes = f"Verified {matches}/{steps} steps match."
    if status == "FAIL":
        notes += f" Drift detected at step {matches}."
        
    return status, notes, results

def main():
    parser = argparse.ArgumentParser(description="LabWired Determinism Audit Tool")
    parser.add_argument("--hw-trace", required=True, help="Path to hardware trace JSON")
    parser.add_argument("--sim-trace", required=True, help="Path to simulation trace JSON")
    parser.add_argument("--target", default="Unknown", help="Target board/MCU name")
    parser.add_argument("--firmware", default="Unknown", help="Firmware name/version")
    parser.add_argument("--output", required=True, help="Path to output report JSON")
    parser.add_argument("--max-steps", type=int, help="Limit number of steps to compare")
    parser.add_argument("--align-window", type=int, default=10, help="Window size for alignment search")
    
    args = parser.parse_args()
    
    try:
        hw_trace = json.loads(Path(args.hw_trace).read_text())
        sim_trace = json.loads(Path(args.sim_trace).read_text())
    except Exception as e:
        print(f"Error loading traces: {e}")
        sys.exit(1)
        
    status, notes, steps_results = compare_traces(hw_trace, sim_trace, args.max_steps, args.align_window)
    
    report = {
        "timestamp": datetime.now().strftime("%a %b %d %H:%M:%S %Y"),
        "target": args.target,
        "firmware": args.firmware,
        "verification_tool": "labwired-audit (v0.1.0)",
        "steps_compared": len(steps_results),
        "status": status,
        "notes": notes,
        "results": steps_results,
        "checksum_verification": {
            "trace_match": "COMPATIBLE" if status == "PASS" else "DRIFT_DETECTED"
        }
    }
    
    with open(args.output, "w") as f:
        json.dump(report, f, indent=2)
        
    print(f"Audit complete. Status: {status}. Report saved to {args.output}")

if __name__ == "__main__":
    main()
