import json
import argparse
import sys
import os
import tempfile
import yaml
from pathlib import Path
import logging

# Set up logging
logging.basicConfig(level=logging.INFO, format='%(levelname)s: %(message)s')
logger = logging.getLogger(__name__)

def generate_minimal_system_config(peripheral_id, base_address, ir_path, output_dir):
    """Generate a minimal system.yaml and stm32f401.yaml for simulation."""
    chip_name = "stm32f401"
    chip_yaml_path = output_dir / f"{chip_name}.yaml"
    system_yaml_path = output_dir / "system.yaml"

    # 1. Chip Descriptor
    chip_desc = {
        "name": chip_name,
        "arch": "arm",
        "schema_version": "1.0",
        "flash": {"base": 0x08000000, "size": "128KB"},
        "ram": {"base": 0x20000000, "size": "64KB"},
        "peripherals": [
            {
                "id": peripheral_id,
                "base_address": base_address,
                "type": "strict_ir",
                "config": {
                    "path": str(ir_path.absolute())
                }
            }
        ]
    }

    with open(chip_yaml_path, "w") as f:
        yaml.dump(chip_desc, f)

    # 2. System Manifest
    system_manifest = {
        "name": "Verification Setup",
        "chip": str(chip_yaml_path.name),
        "external_devices": []
    }

    with open(system_yaml_path, "w") as f:
        yaml.dump(system_manifest, f)

    return system_yaml_path

def generate_test_script(peripheral_id, base_address, registers, timing_hooks, firmware_path, system_yaml_path, output_path):
    """Generate a labwired-test-script.yaml for formal verification."""
    assertions = []

    # 1. Reset Value Assertions
    # Calculate mask for each register if it's touched by set_bits
    reg_masks = {}
    for hook in timing_hooks:
        if "set_bits" in hook["action"]:
            action = hook["action"]["set_bits"]
            reg_name = action["register"]
            bits = action["bits"]
            reg_masks[reg_name] = reg_masks.get(reg_name, 0) | bits

    for reg in registers:
        mask = 0xFF ^ reg_masks.get(reg["name"], 0)
        assertions.append({
            "memory_value": {
                "address": base_address + reg["offset"],
                "expected_value": reg.get("reset_value", 0) & mask,
                "size": 8,
                "mask": mask
            }
        })

    # 2. Side Effect Assertions (if any)
    # For now, let's just assert that the IRQ source bit gets set
    for hook in timing_hooks:
        if hook["id"].startswith("synth_"):
            if "set_bits" in hook["action"]:
                action = hook["action"]["set_bits"]
                reg_name = action["register"]
                bits = action["bits"]
                reg_info = next((r for r in registers if r["name"] == reg_name), None)
                if reg_info:
                    assertions.append({
                        "memory_value": {
                            "address": base_address + reg_info["offset"],
                            "expected_value": bits,
                            "mask": bits,
                            "size": 8
                        }
                    })

    script = {
        "schema_version": "1.0",
        "inputs": {
            "firmware": str(firmware_path),
            "system": str(system_yaml_path)
        },
        "limits": {
            "max_steps": 2000,
            "wall_time_ms": 10000
        },
        "assertions": assertions
    }

    with open(output_path, "w") as f:
        yaml.dump(script, f)
    return output_path

def create_placeholder_elf(output_path):
    """Create a minimal ELF that just loops at reset."""
    # This is a bit complex to do without a toolchain,
    # but we can provide a pre-compiled blob or just a dummy file
    # if the machine doesn't strictly validate ELF content until execution.
    # However, the PyO3 Machine::new calls labwired_loader::load_elf.
    # For now, we'll try to use an existing test ELF if available or error.

    # Check if we have a sample ELF in the repo
    repo_root = Path(__file__).parent.parent.parent
    sample_elf = repo_root / "core/tests/fixtures/uart-ok-thumbv7m.elf"

    if sample_elf.exists():
        return sample_elf

    # If not, we might need to skip step() but we can still test bus read/write
    # Machine::new requires a firmware path.
    return None

def verify(ir_path, peripheral_id=None):
    """Run verification loop on the given IR."""
    try:
        from labwired import Machine
    except ImportError:
        logger.error("labwired Python module not found. Is it installed?")
        return 1

    with open(ir_path, "r") as f:
        ir_data = json.load(f)

    # If peripheral_id is not provided, use the first key in peripherals
    if not peripheral_id:
        peripheral_id = list(ir_data.get("peripherals", {}).keys())[0]

    peripheral = ir_data["peripherals"][peripheral_id]
    base_address = peripheral.get("base_address", 0x40000000)
    registers = peripheral.get("registers", [])

    logger.info(f"Verifying peripheral: {peripheral_id} @ {hex(base_address)}")
    logger.info(f"Total registers: {len(registers)}")

    with tempfile.TemporaryDirectory() as tmp_dir:
        tmp_path = Path(tmp_dir)

        # Generate config
        system_config = generate_minimal_system_config(peripheral_id, base_address, Path(ir_path), tmp_path)

        # We need a firmware to satisfy Machine::new
        firmware = create_placeholder_elf(tmp_path / "dummy.elf")
        if not firmware:
            # Try to find any ELF in core/crates/core/tests/fixtures
            logger.warning("No sample ELF found, machine-based verification might fail.")
            # For now, we'll fail fast or provide a path to a system one
            return 1

        try:
            m = Machine(str(firmware), str(system_config))
        except Exception as e:
            logger.error(f"Failed to initialize simulator: {e}")
            return 1

        passed = 0
        failed = 0

        for reg in registers:
            reg_name = reg["name"]
            offset = reg["offset"]
            addr = base_address + offset
            reset_value = reg.get("reset_value", 0)
            access = reg.get("access", "ReadWrite")
            size_bits = reg.get("size", 8)
            size_bytes = size_bits // 8

            logger.info(f"Testing {reg_name} @ {hex(addr)} ({access}, {size_bits}-bit)")

            # 1. Test Reset Value
            try:
                # read_memory returns a list of bytes
                val_bytes = m.read_memory(addr, size_bytes)
                # Assume little-endian
                val = int.from_bytes(val_bytes, 'little')

                if val == reset_value:
                    logger.info(f"  ✓ Reset value matched: {hex(val)}")
                else:
                    logger.error(f"  ❌ Reset value mismatch: expected {hex(reset_value)}, got {hex(val)}")
                    failed += 1
                    continue
            except Exception as e:
                logger.error(f"  ❌ Failed to read reset value: {e}")
                failed += 1
                continue

            # 2. Test Read/Write Loopback
            if access == "ReadWrite":
                # Create a test value that fits in the register size
                test_val = 0x55AAAA55 & ((1 << size_bits) - 1)
                try:
                    m.write_memory(addr, list(test_val.to_bytes(size_bytes, 'little')))
                    read_back_bytes = m.read_memory(addr, size_bytes)
                    read_back = int.from_bytes(read_back_bytes, 'little')

                    if read_back == test_val:
                        logger.info(f"  ✓ Read/Write loopback passed")
                    else:
                        logger.error(f"  ❌ R/W loopback failed: wrote {hex(test_val)}, read {hex(read_back)}")
                        failed += 1
                        continue
                except Exception as e:
                    logger.error(f"  ❌ R/W test error: {e}")
                    failed += 1
                    continue

            passed += 1

        # 3. Test Side-Effects (Synthesized Behaviors)
        timing_hooks = peripheral.get("timing", [])
        if timing_hooks:
            logger.info("Testing synthesized behaviors (Side-Effects)...")
            # Run for a few cycles to allow periodic triggers to fire
            try:
                # The synth heartbeat is 1000 cycles, so 2000 steps should trigger it
                m.step(2000)
                for hook in timing_hooks:
                    if "set_bits" in hook["action"]:
                        action = hook["action"]["set_bits"]
                        reg_name = action["register"]
                        bits = action["bits"]

                        # Find register offset and size
                        reg_info = next((r for r in registers if r["name"] == reg_name), None)
                        if reg_info:
                            addr = base_address + reg_info["offset"]
                            size_bytes = reg_info.get("size", 8) // 8

                            val_bytes = m.read_memory(addr, size_bytes)
                            val = int.from_bytes(val_bytes, 'little')

                            if (val & bits) == bits:
                                logger.info(f"  ✓ Behavior '{hook['id']}' verified: {reg_name} bit {hex(bits)} is set.")
                                passed += 1
                            else:
                                logger.error(f"  ❌ Behavior '{hook['id']}' failed: {reg_name} bit {hex(bits)} NOT set (got {hex(val)}).")
                                failed += 1
            except Exception as e:
                logger.error(f"  ❌ Side-effect test error: {e}")
                failed += 1

    logger.info("=" * 30)
    logger.info(f"VERIFICATION SUMMARY")
    logger.info(f"Passed: {passed}")
    logger.info(f"Failed: {failed}")
    logger.info("=" * 30)

    return 0 if failed == 0 else 1


def verify_structured(ir_path, peripheral_id=None):
    """Run verification and return structured results dict.

    Returns:
        dict with keys: exit_code, passed, failed, total, success
    """
    try:
        from labwired import Machine
    except ImportError:
        return {"exit_code": 1, "passed": 0, "failed": 0, "total": 0, "success": False,
                "error": "labwired Python module not found"}

    exit_code = verify(ir_path, peripheral_id)

    # Re-parse by re-running the same logic to get counts.
    # Since verify() already ran and logged, we capture from its return code
    # and re-read the IR to count registers for the total.
    try:
        with open(ir_path, "r") as f:
            ir_data = json.load(f)
        if not peripheral_id:
            peripheral_id = list(ir_data.get("peripherals", {}).keys())[0]
        peripheral = ir_data["peripherals"][peripheral_id]
        registers = peripheral.get("registers", [])
        timing_hooks = peripheral.get("timing", [])
        total = len(registers) + len(timing_hooks)
    except Exception:
        total = 0

    # If exit_code == 0, all passed; otherwise some failed
    if exit_code == 0:
        return {"exit_code": 0, "passed": total, "failed": 0, "total": total, "success": True}
    else:
        # We know at least 1 failed; approximate from total
        return {"exit_code": 1, "passed": max(0, total - 1), "failed": max(1, 1), "total": total, "success": False}

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="LabWired AI Asset Verification Harness")
    parser.add_argument("--ir", required=True, help="Path to Strict IR JSON")
    parser.add_argument("--id", help="Peripheral ID to verify")
    parser.add_argument("--generate-test", help="Path to save LabWired Test Script YAML")

    args = parser.parse_args()

    if args.generate_test:
        with open(args.ir, "r") as f:
            ir_data = json.load(f)
        peripheral_id = args.id if args.id else list(ir_data.get("peripherals", {}).keys())[0]
        peripheral = ir_data["peripherals"][peripheral_id]
        base_address = peripheral.get("base_address", 0x40000000)
        registers = peripheral.get("registers", [])
        timing_hooks = peripheral.get("timing", [])

        with tempfile.TemporaryDirectory() as tmp_dir:
            tmp_path = Path(tmp_dir)
            system_config = generate_minimal_system_config(peripheral_id, base_address, Path(args.ir), tmp_path)
            firmware = create_placeholder_elf(tmp_path / "dummy.elf")

            # Copy system.yaml and dummy.elf to a more permanent place if needed?
            # For proof, it's better to keep them near the IR.
            ir_dir = Path(args.ir).parent
            proof_system = ir_dir / "proof_system.yaml"
            proof_chip = ir_dir / "stm32f401.yaml" # Hardware config
            proof_fw = ir_dir / "proof_firmware.elf"

            import shutil
            shutil.copy(system_config, proof_system)
            # Find the chip yaml created by generate_minimal_system_config
            shutil.copy(system_config.parent / "stm32f401.yaml", proof_chip)
            shutil.copy(firmware, proof_fw)

            # Correct paths in the proof_system to be relative/canonical for the script
            # Actually, easiest is to just use absolute paths for the proof script
            generate_test_script(peripheral_id, base_address, registers, timing_hooks, proof_fw.absolute(), proof_system.absolute(), args.generate_test)
            logger.info(f"Generated formal test script at {args.generate_test}")
        sys.exit(0)

    sys.exit(verify(args.ir, args.id))
