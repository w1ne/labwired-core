import sys
import os

# Example Usage of LabWired Python Bindings
# Usage: python test_agent.py <firmware.elf>

try:
    import labwired
except ImportError:
    print("Error: 'labwired' module not found.")
    print("Did you run 'maturin develop' in crates/python?")
    sys.exit(1)

def main():
    if len(sys.argv) < 2:
        print("Usage: python test_agent.py <firmware.elf>")
        sys.exit(1)
        
    firmware_path = sys.argv[1]
    
    print(f"Initializing Machine with {firmware_path}...")
    try:
        machine = labwired.Machine(firmware_path)
    except Exception as e:
        print(f"Failed to load machine: {e}")
        sys.exit(1)
        
    print(f"Initial PC: {machine.get_pc():#x}")
    
    print("Stepping 100 instructions...")
    reason = machine.step(100)
    print(f"Stop Reason: {reason}")
    print(f"Current PC: {machine.get_pc():#x}")
    
    # Inspect Registers (R0-R12 are 0-12)
    r0 = machine.read_register(0)
    print(f"R0: {r0:#x}")
    
    # Snapshot Test
    print("Taking Snapshot...")
    snap_json = machine.snapshot()
    print(f"Snapshot taken ({len(snap_json)} bytes)")
    
    # Step more
    print("Stepping 50 more...")
    machine.step(50)
    pc_after = machine.get_pc()
    print(f"PC after 50 steps: {pc_after:#x}")
    
    # Restore
    print("Restoring Snapshot...")
    machine.restore(snap_json)
    pc_restored = machine.get_pc()
    print(f"PC restored: {pc_restored:#x}")
    
    if pc_restored != pc_after:
        print("Time Travel Successful! PC reverted.")
    else:
        print("Warning: PC did not change (or restored to same state).")

if __name__ == "__main__":
    main()
