#!/usr/bin/env python3
"""
ADXL345 Register Map Verification Script
Compares AI-generated YAML against ground truth from datasheet.
"""

import json
import yaml
import sys
from pathlib import Path

def normalize_offset(offset_str):
    """Normalize offset format (0x1D, 0x1d, 1D all become 0x1D)"""
    if isinstance(offset_str, int):
        return f"0x{offset_str:02X}"
    offset_str = str(offset_str).strip().upper()
    if not offset_str.startswith('0X'):
        offset_str = '0X' + offset_str
    return offset_str

def normalize_access(access_str):
    """Normalize access mode strings"""
    if not access_str:
        return "UNKNOWN"
    access_map = {
        'READONLY': 'RO',
        'READ_ONLY': 'RO',
        'RO': 'RO',
        'READWRITE': 'RW',
        'READ_WRITE': 'RW',
        'R/W': 'RW',
        'RW': 'RW',
        'WRITEONLY': 'WO',
        'WRITE_ONLY': 'WO',
        'WO': 'WO'
    }
    return access_map.get(str(access_str).upper().replace(' ', ''), 'UNKNOWN')

def verify_register_map(ground_truth_path, generated_yaml_path):
    """Verify register offsets, reset values, and access modes"""

    with open(ground_truth_path) as f:
        truth = json.load(f)

    with open(generated_yaml_path) as f:
        generated = yaml.safe_load(f)

    errors = []
    warnings = []

    # Check all expected registers are present
    truth_regs = {r['name'].upper(): r for r in truth['registers']}
    gen_regs = {r['name'].upper(): r for r in generated['registers']}

    # Missing registers
    missing = set(truth_regs.keys()) - set(gen_regs.keys())
    if missing:
        errors.append(f"Missing registers: {', '.join(sorted(missing))}")

    # Extra registers (not in datasheet)
    extra = set(gen_regs.keys()) - set(truth_regs.keys())
    if extra:
        warnings.append(f"Extra registers not in datasheet: {', '.join(sorted(extra))}")

    # Verify each register
    for reg_name, truth_reg in truth_regs.items():
        if reg_name not in gen_regs:
            continue  # Already reported as missing

        gen_reg = gen_regs[reg_name]

        # Check offset
        truth_offset = normalize_offset(truth_reg['offset'])
        gen_offset = normalize_offset(gen_reg['offset'])
        if truth_offset != gen_offset:
            errors.append(f"{reg_name}: Offset mismatch - expected {truth_offset}, got {gen_offset}")

        # Check reset value
        truth_reset = normalize_offset(truth_reg['reset'])
        gen_reset = normalize_offset(gen_reg.get('reset_value', '0x00'))
        if truth_reset != gen_reset:
            errors.append(f"{reg_name}: Reset value mismatch - expected {truth_reset}, got {gen_reset}")

        # Check access mode
        truth_access = normalize_access(truth_reg['access'])
        gen_access = normalize_access(gen_reg.get('access', 'UNKNOWN'))
        if truth_access != gen_access:
            errors.append(f"{reg_name}: Access mode mismatch - expected {truth_access}, got {gen_access}")

    return errors, warnings

def main():
    script_dir = Path(__file__).parent
    ground_truth = script_dir / "adxl345_ground_truth.json"
    generated_yaml = script_dir / "adxl345_gen.yaml"

    print("=" * 70)
    print("ADXL345 Register Map Verification")
    print("=" * 70)
    print(f"Ground Truth: {ground_truth}")
    print(f"Generated:    {generated_yaml}")
    print()

    errors, warnings = verify_register_map(ground_truth, generated_yaml)

    if warnings:
        print("⚠️  WARNINGS:")
        for w in warnings:
            print(f"  - {w}")
        print()

    if errors:
        print("❌ ERRORS FOUND:")
        for e in errors:
            print(f"  - {e}")
        print()
        print(f"Total Errors: {len(errors)}")
        return 1
    else:
        print("✅ ALL CHECKS PASSED")
        print("   - All 30 registers present")
        print("   - All offsets match datasheet")
        print("   - All reset values correct")
        print("   - All access modes correct")
        return 0

if __name__ == "__main__":
    sys.exit(main())
