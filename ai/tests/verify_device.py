#!/usr/bin/env python3
"""
Generic verification script that can verify any device.
Usage: python3 verify_device.py <device_name>
"""

import json
import yaml
import sys
from pathlib import Path

def normalize_offset(offset_str):
    """Normalize offset format"""
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
        'READONLY': 'RO', 'READ_ONLY': 'RO', 'RO': 'RO',
        'READWRITE': 'RW', 'READ_WRITE': 'RW', 'R/W': 'RW', 'RW': 'RW',
        'WRITEONLY': 'WO', 'WRITE_ONLY': 'WO', 'WO': 'WO'
    }
    return access_map.get(str(access_str).upper().replace(' ', ''), 'UNKNOWN')

def verify_device(device_name):
    """Verify a device's register map and bitfields"""
    script_dir = Path(__file__).parent
    ground_truth = script_dir / f"{device_name}_ground_truth.json"
    bitfields_truth = script_dir / f"{device_name}_bitfields_truth.json"
    generated_yaml = script_dir / f"{device_name}_gen.yaml"

    if not ground_truth.exists():
        print(f"❌ Ground truth file not found: {ground_truth}")
        return 1

    if not generated_yaml.exists():
        print(f"❌ Generated YAML not found: {generated_yaml}")
        return 1

    with open(ground_truth) as f:
        truth = json.load(f)

    with open(generated_yaml) as f:
        generated = yaml.safe_load(f)

    print("=" * 70)
    print(f"{truth['device']} Verification Report")
    print("=" * 70)
    print(f"Ground Truth: {ground_truth}")
    print(f"Generated:    {generated_yaml}")
    print()

    errors = []
    warnings = []

    # Verify register map
    truth_regs = {r['name'].upper(): r for r in truth['registers']}
    gen_regs = {r['name'].upper(): r for r in generated['registers']}

    missing = set(truth_regs.keys()) - set(gen_regs.keys())
    if missing:
        errors.append(f"Missing registers: {', '.join(sorted(missing))}")

    extra = set(gen_regs.keys()) - set(truth_regs.keys())
    if extra:
        warnings.append(f"Extra registers: {', '.join(sorted(extra))}")

    for reg_name, truth_reg in truth_regs.items():
        if reg_name not in gen_regs:
            continue

        gen_reg = gen_regs[reg_name]

        truth_offset = normalize_offset(truth_reg['offset'])
        gen_offset = normalize_offset(gen_reg['offset'])
        if truth_offset != gen_offset:
            errors.append(f"{reg_name}: Offset mismatch - expected {truth_offset}, got {gen_offset}")

        truth_reset = normalize_offset(truth_reg['reset'])
        gen_reset = normalize_offset(gen_reg.get('reset_value', '0x00'))
        if truth_reset != gen_reset:
            errors.append(f"{reg_name}: Reset value mismatch - expected {truth_reset}, got {gen_reset}")

        truth_access = normalize_access(truth_reg['access'])
        gen_access = normalize_access(gen_reg.get('access', 'UNKNOWN'))
        if truth_access != gen_access:
            errors.append(f"{reg_name}: Access mode mismatch - expected {truth_access}, got {gen_access}")

    # Verify bitfields if ground truth exists
    if bitfields_truth.exists():
        with open(bitfields_truth) as f:
            bf_truth = json.load(f)

        for reg_name, truth_fields in bf_truth['bitfields'].items():
            reg_name_upper = reg_name.upper()

            if reg_name_upper not in gen_regs:
                errors.append(f"{reg_name}: Register not found for bitfield verification")
                continue

            gen_reg = gen_regs[reg_name_upper]
            gen_fields = {f['name'].upper().replace(' ', '_').replace('-', '_'): f
                         for f in gen_reg.get('fields', [])}

            for field_name, truth_field in truth_fields.items():
                field_name_norm = field_name.upper().replace(' ', '_').replace('-', '_')

                if field_name_norm not in gen_fields:
                    errors.append(f"{reg_name}.{field_name}: Field not found")
                    continue

                gen_field = gen_fields[field_name_norm]
                truth_bits = truth_field['bits']
                gen_bits = gen_field.get('bit_range', [])

                if len(gen_bits) != 2:
                    errors.append(f"{reg_name}.{field_name}: Invalid bit_range format")
                    continue

                if gen_bits != truth_bits:
                    errors.append(
                        f"{reg_name}.{field_name}: Bit range mismatch - "
                        f"expected [{truth_bits[0]}, {truth_bits[1]}], "
                        f"got [{gen_bits[0]}, {gen_bits[1]}]"
                    )

    # Print results
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
        print(f"   - All {len(truth_regs)} registers verified")
        print("   - All offsets match datasheet")
        print("   - All reset values correct")
        print("   - All access modes correct")
        if bitfields_truth.exists():
            print("   - All bitfields verified")
        return 0

if __name__ == "__main__":
    if len(sys.argv) != 2:
        print("Usage: python3 verify_device.py <device_name>")
        print("Example: python3 verify_device.py adxl345")
        sys.exit(1)

    sys.exit(verify_device(sys.argv[1]))
