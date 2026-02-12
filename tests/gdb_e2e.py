import subprocess
import time
import os
import signal
from pygdbmi.gdbcontroller import GdbController

# Configuration
LABWIRED_BIN = "/home/andrii/Projects/labwired/core/target/release/labwired"
FIRMWARE_BIN = "/home/andrii/Projects/labwired/core/target/thumbv7m-none-eabi/debug/demo-blinky"
GDB_PORT = 3333

def test_gdb_sticky_breakpoint():
    print("Starting LabWired GDB E2E Test...")

    # 1. Start LabWired in GDB mode
    # --gdb port --firmware path
    cmd = [LABWIRED_BIN, "--gdb", str(GDB_PORT), "--firmware", FIRMWARE_BIN]
    process = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)

    time.sleep(2)  # Give it a bit more time to start

    gdb = None
    try:
        # Check if process is still running
        if process.poll() is not None:
             out, err = process.communicate()
             print(f"ERROR: LabWired exited prematurely with code {process.returncode}")
             print(f"STDOUT: {out}")
             print(f"STDERR: {err}")
             raise Exception(f"LabWired exited with code {process.returncode}")

        # 2. Start GDB and connect
        gdb = GdbController(command=["/usr/bin/gdb-multiarch", "--interpreter=mi3"])
        print(f"Connecting to : {GDB_PORT}...")
        resp = gdb.write(f"target remote :{GDB_PORT}", timeout_sec=5.0)
        print(f"Connect Response: {resp}")

        # 3. Set a breakpoint at main
        print("Setting breakpoint at 0x0...")
        resp = gdb.write("break *0x0")
        print(f"Break Response: {resp}")

        # 4. Continue
        print("Continuing...")
        resp = gdb.write("continue")
        print(f"Continue Response: {resp}")

        # Wait for stop
        stop_found = False
        for i in range(20):
            response = gdb.get_gdb_response(timeout_sec=0.5)
            # print(f"GDB Response: {response}") # Too verbose, but useful if needed
            for r in response:
                if r['message'] == 'stopped':
                     stop_found = True
                     print(f"STOPPED at: {r['payload'].get('frame', {}).get('addr', 'unknown')}")
                     break
            if stop_found: break
            time.sleep(0.5)

        if not stop_found:
             out, err = process.communicate()
             print(f"ERROR: Failed to hit initial breakpoint. LabWired output:")
             print(f"STDOUT: {out}")
             print(f"STDERR: {err}")
             assert False, "Failed to hit initial breakpoint"

        # 5. Continue AGAIN (The Sticky Fix Test)
        print("Continuing again (Sticky Fix Test)...")
        gdb.write("continue")

        # It should NOT stop immediately at the same address if the fix works.
        # We expect it to run or stop at next BP if we set one.
        # For now, we just check that it doesn't immediately 'stop' at 0x0 again.

        time.sleep(1)
        response = gdb.get_gdb_response(timeout_sec=0.1)

        stopped_at_same = False
        for r in response:
             if r['message'] == 'stopped':
                  # Check address if possible
                  stopped_at_same = True

        assert not stopped_at_same, "STUCK! Debugger stopped immediately at the same breakpoint."
        print("Passed Sticky Fix Test: Debugger moved past breakpoint.")

    finally:
        if gdb:
            gdb.exit()
        process.terminate()
        process.wait()
        print("Test Cleaned Up.")

if __name__ == "__main__":
    try:
        test_gdb_sticky_breakpoint()
        print("\nALL GDB E2E TESTS PASSED.")
    except Exception as e:
        print(f"\nTEST FAILED: {e}")
        exit(1)
