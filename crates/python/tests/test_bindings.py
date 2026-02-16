import pytest
import labwired
import os
import json

# This fixture assumes we have access to a test binary.
# In a real CI, we would build `firmware-ci-fixture` or `demo-blinky`.
# For now, we expect the user to provide FIRMWARE_PATH env var or we look in default places.
FIRMWARE_PATH = os.environ.get("LABWIRED_FIRMWARE", "../../examples/demo-blinky/target/thumbv7em-none-eabihf/debug/demo-blinky")

@pytest.fixture
def machine():
    if not os.path.exists(FIRMWARE_PATH):
        pytest.skip(f"Firmware not found at {FIRMWARE_PATH}")
    return labwired.Machine(FIRMWARE_PATH)

def test_initial_state(machine):
    # Cortex-M Reset Vector is usually at 0x00000004 or 0x08000004
    # The PC should be non-zero.
    pc = machine.get_pc()
    assert pc > 0, "PC should be initialized from reset vector"

def test_register_access(machine):
    # R0 is usually a scratch register
    machine.write_register(0, 0xDEADBEEF)
    assert machine.read_register(0) == 0xDEADBEEF

    # R1
    machine.write_register(1, 0xCAFEBABE)
    assert machine.read_register(1) == 0xCAFEBABE

def test_memory_access(machine):
    # Write to RAM (usually 0x20000000)
    ram_addr = 0x20000000
    data = [0x11, 0x22, 0x33, 0x44]
    
    machine.write_memory(ram_addr, data)
    read_back = machine.read_memory(ram_addr, 4)
    
    assert list(read_back) == data

def test_execution_steps(machine):
    start_pc = machine.get_pc()
    reason = machine.step(10)
    end_pc = machine.get_pc()
    
    assert reason.kind == "max_steps_reached"
    # PC should change (unless it's an infinite loop on same instruction, which is rare for 10 steps from reset)
    assert end_pc != start_pc

def test_snapshot_restore(machine):
    # 1. Run a bit
    machine.step(50)
    pc_before = machine.get_pc()
    
    # 2. Modify state
    machine.write_register(0, 0x12345678)
    
    # 3. Snapshot
    snapshot_json = machine.snapshot()
    snapshot = json.loads(snapshot_json)
    assert "cpu" in snapshot
    
    # 4. Modify state again (mess it up)
    machine.step(10)
    machine.write_register(0, 0x00000000)
    assert machine.read_register(0) == 0
    assert machine.get_pc() != pc_before
    
    # 5. Restore
    machine.restore(snapshot_json)
    
    # 6. Verify restored state
    assert machine.get_pc() == pc_before
    assert machine.read_register(0) == 0x12345678

def test_performance_benchmark(machine):
    """
    Test raw stepping performance. In release builds, this should be fast.
    """
    import time
    
    steps = 100_000
    start = time.time()
    machine.step(steps)
    end = time.time()
    
    elapsed = end - start
    ips = steps / elapsed
    print(f"\n[Benchmark] {steps} steps in {elapsed:.4f}s => {ips:,.0f} instructions/sec")
    
    # Very loose assertion just to ensure it runs, real performance depends on host
    assert elapsed < 10.0, "Should handle 100k steps reasonably fast"
