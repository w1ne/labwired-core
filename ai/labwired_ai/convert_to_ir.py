import yaml
import json
import sys
import re

def parse_hex(s):
    if not s or (isinstance(s, str) and s.lower() == 'n/a'):
        return 0
    if isinstance(s, int):
        return s
    s = str(s).strip()
    if not s:
        return 0
    # Handle binary strings like '00001010'
    if len(s) == 8 and all(c in '01' for c in s):
        return int(s, 2)
    try:
        if s.lower().startswith('0x'):
            return int(s, 16)
        return int(s, 10)
    except ValueError:
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

        # Calculate register size based on fields
        reg_size = 8
        if fields:
            max_bit = max(f['bit_offset'] + f['bit_width'] for f in fields)
            if max_bit <= 8:
                reg_size = 8
            elif max_bit <= 16:
                reg_size = 16
            else:
                reg_size = 32

        registers.append({
            "name": sanitize_ident(r.get('name', 'UNKNOWN')),
            "offset": parse_hex(r.get('offset', '0x00')),
            "size": reg_size,
            "access": map_access(r.get('access', 'ReadWrite')),
            "reset_value": parse_hex(r.get('reset_value', '0x00')),
            "fields": fields,
            "side_effects": r.get('side_effects'),
            "description": r.get('description', '')
        })

    peripheral_name = sanitize_ident(data.get('name', 'SENSOR'))

    # Map AI behaviors (side_effects list in YAML) to timing hooks
    timing_hooks = []
    interrupt_mapping = {}
    irq_counter = 10 # Start IRQ numbers at 10

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
        if t.get("interrupt") and t.get("interrupt") not in interrupt_mapping:
            interrupt_mapping[t.get("interrupt")] = irq_counter
            irq_counter += 1

    # Heuristic Synthesis: If we find something that looks like an interrupt status register,
    # and no behavior is defined, synthesize a periodic "data ready" heartbeat.
    if not timing_hooks:
        for reg in registers:
            name_upper = reg['name'].upper()
            if 'INT' in name_upper and ('SOURCE' in name_upper or 'STATUS' in name_upper):
                # Synthesize a periodic data ready event (bit 7 is a common convention)
                hook_id = f"synth_{reg['name']}_heartbeat"
                irq_name = f"{reg['name'].upper()}_IRQ"
                timing_hooks.append({
                    "id": hook_id,
                    "trigger": {"periodic": {"period_cycles": 10}},
                    "delay_cycles": 0,
                    "action": {"set_bits": {"register": reg['name'], "bits": 0x80}},
                    "interrupt": irq_name
                })
                if irq_name not in interrupt_mapping:
                    interrupt_mapping[irq_name] = irq_counter
                    irq_counter += 1
                # Only synthesize one heartbeat for now to avoid flooding
                break

    peripheral = {
        "name": peripheral_name,
        "base_address": 0x40000000, # Arbitrary base address for codegen
        "description": "AI Generated Peripheral",
        "registers": registers,
        "interrupts": [{"name": name, "value": val} for name, val in interrupt_mapping.items()],
        "timing": timing_hooks
    }


    # peripherals[peripheral_name] = peripheral # This line is removed

    output_data = {
        "name": data.get('name', 'unknown').lower(),
        "arch": "Arm", # Default to Arm
        "description": "AI Generated Device for Codegen",
        "peripherals": {
            peripheral['name']: peripheral
        },
        "interrupt_mapping": interrupt_mapping
    }

    with open(output_path, 'w') as f:
        json.dump(output_data, f, indent=2)

if __name__ == "__main__":
    if len(sys.argv) < 3:
        print("Usage: python3 convert_to_ir.py <input.yaml> <output.json>")
        sys.exit(1)
    convert(sys.argv[1], sys.argv[2])
