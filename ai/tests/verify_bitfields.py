#!/usr/bin/env python3
"""
ADXL345 Bitfield Verification Script
Verifies bit positions and ranges for all register fields.
"""

import json
import yaml
import sys
from pathlib import Path

def normalize_field_name(name):
    """Normalize field names for comparison"""
    return str(name).upper().replace(' ', '_').replace('-', '_')

def verify_bitfields(ground_truth_path, generated_yaml_path):
    """Verify bitfield positions match datasheet"""

    with open(ground_truth_path) as f:
        truth = json.load(f)

    with open(generated_yaml_path) as f:
        generated = yaml.safe_load(f)

    errors = []
    warnings = []

    # Build lookup for generated registers
    gen_regs = {r['name'].upper(): r for r in generated['registers']}

    # Check each register's bitfields
    for reg_name, truth_fields in truth['bitfields'].items():
        reg_name_upper = reg_name.upper()

        if reg_name_upper not in gen_regs:
            errors.append(f"{reg_name}: Register not found in generated YAML")
            continue

        gen_reg = gen_regs[reg_name_upper]
        gen_fields = {normalize_field_name(f['name']): f for f in gen_reg.get('fields', [])}

        # Check each field
        for field_name, truth_field in truth_fields.items():
            field_name_norm = normalize_field_name(field_name)

            if field_name_norm not in gen_fields:
                errors.append(f"{reg_name}.{field_name}: Field not found in generated YAML")
                continue

            gen_field = gen_fields[field_name_norm]
            truth_bits = truth_field['bits']
            gen_bits = gen_field.get('bit_range', [])

            # Verify bit range
            if len(gen_bits) != 2:
                errors.append(f"{reg_name}.{field_name}: Invalid bit_range format: {gen_bits}")
                continue

            # Check if bit positions match
            if gen_bits != truth_bits:
                errors.append(
                    f"{reg_name}.{field_name}: Bit range mismatch - "
                    f"expected [{truth_bits[0]}, {truth_bits[1]}], "
                    f"got [{gen_bits[0]}, {gen_bits[1]}]"
                )

    return errors, warnings

def main():
    script_dir = Path(__file__).parent
    ground_truth = script_dir / "adxl345_bitfields_truth.json"
    generated_yaml = script_dir / "adxl345_gen.yaml"

    print("=" * 70)
    print("ADXL345 Bitfield Verification")
    print("=" * 70)
    print(f"Ground Truth: {ground_truth}")
    print(f"Generated:    {generated_yaml}")
    print()

    errors, warnings = verify_bitfields(ground_truth, generated_yaml)

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
        print("✅ ALL BITFIELD CHECKS PASSED")
        print("   - All critical register bitfields verified")
        print("   - All bit positions match datasheet")
        print("   - All bit ranges correct")
        return 0

if __name__ == "__main__":
    sys.exit(main())
