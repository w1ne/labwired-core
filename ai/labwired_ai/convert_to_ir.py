import yaml
import json
import sys
import re

def parse_hex(s):
    if not s or (isinstance(s, str) and s.lower() == 'n/a'):
        return 0
    if isinstance(s, int):
        return s
    try:
        return int(s, 16)
    except ValueError:
        return 0

def map_access(s):
    if not s:
        return "ReadWrite"
    s = s.lower()
    if 'readwrite' in s or 'r/w' in s:
        return "ReadWrite"
    if 'readonly' in s or 'ro' in s:
        return "ReadOnly"
    if 'writeonly' in s or 'wo' in s:
        return "WriteOnly"
    return "Unknown"

def sanitize_ident(s):
    if not s:
        return 'unknown'
    # Convert to snake_case and remove invalid characters
    s = str(s).replace(' ', '_').replace('-', '_').replace('[', '_').replace(']', '_').replace(':', '_')
    s = re.sub(r'[^a-zA-Z0-9_]', '', s)
    if s and s[0].isdigit():
        s = 'reg_' + s
    if not s:
        s = 'unknown'
    return s.lower()

def convert(input_path, output_path):
    with open(input_path, 'r') as f:
        data = yaml.safe_load(f)

    peripherals = {}

    # Create an IrPeripheral
    registers = []
    for r in data.get('registers', []):
        fields = []
        for f in r.get('fields', []):
            bit_range = f.get('bit_range', [0, 0])
            low = min(bit_range)
            high = max(bit_range)
            fields.append({
                "name": sanitize_ident(f.get('name', 'UNKNOWN')),
                "bit_offset": low,
                "bit_width": high - low + 1,
                "access": None,
                "description": f.get('description', '')
            })

        registers.append({
            "name": sanitize_ident(r.get('name', 'UNKNOWN')),
            "offset": parse_hex(r.get('offset', '0x00')),
            "size": 32, # Default to 32 bits for the container
            "access": map_access(r.get('access', 'ReadWrite')),
            "reset_value": parse_hex(r.get('reset_value', '0x00')),
            "fields": fields,
            "side_effects": r.get('side_effects'),
            "description": r.get('description', '')
        })

    peripheral_name = sanitize_ident(data.get('name', 'SENSOR'))

    # Map AI behaviors (side_effects list in YAML) to timing hooks
    timing_hooks = []
    for t in data.get('side_effects', []):
        # Strip AI-only reasoning/evidence fields for the IR
        hook = {
            "id": t.get("id", "behavior"),
            "trigger": t.get("trigger"),
            "delay_cycles": int(t.get("delay_cycles", 0)),
            "action": t.get("action"),
            "interrupt": t.get("interrupt")
        }
        timing_hooks.append(hook)

    peripheral = {
        "name": peripheral_name,
        "base_address": 0x40000000, # Arbitrary base address for codegen
        "description": "AI Generated Peripheral",
        "registers": registers,
        "interrupts": [],
        "timing": timing_hooks
    }


    peripherals[peripheral_name] = peripheral

    device = {
        "name": sanitize_ident(data.get('name', 'DEVICE')),
        "arch": "Arm",
        "description": "AI Generated Device for Codegen",
        "peripherals": peripherals,
        "interrupt_mapping": {}
    }

    with open(output_path, 'w') as f:
        json.dump(device, f, indent=2)

if __name__ == "__main__":
    if len(sys.argv) < 3:
        print("Usage: python3 convert_to_ir.py <input.yaml> <output.json>")
        sys.exit(1)
    convert(sys.argv[1], sys.argv[2])
