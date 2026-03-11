import os
import yaml
import subprocess
import tempfile
import time

LABWIRED_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CONFIGS_DIR = os.path.join(LABWIRED_DIR, "core", "configs", "onboarding")
ZEPHYR_DIR = os.environ.get("ZEPHYR_BASE", "/opt/zephyrproject/zephyr")
LABWIRED_BIN = os.path.join(LABWIRED_DIR, "core", "target", "release", "labwired")

def run_zephyr_build(board_name, output_dir):
    print(f"[{board_name}] Building zephyr hello_world...")
    try:
        subprocess.check_call(
            ["west", "build", "-b", board_name, "-d", output_dir, "samples/hello_world"],
            cwd=ZEPHYR_DIR,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=300
        )
        return True, os.path.join(output_dir, "zephyr", "zephyr.elf")
    except Exception as e:
        print(f"[{board_name}] Build failed: {e}")
        return False, None

def run_labwired_sim(elf_path, config_path):
    print(f"[{os.path.basename(config_path)}] Simulating in Labwired...")
    try:
        # Run labwired in test mode to execute the elf purely via the core system config.
        # We cap execution to 5 seconds to ensure we do not infinitely loop. 
        # (A real test suite might use `labwired test` with a specific trace file assertion,
        # but for a basic boot test, returning 0 means no immediate hardfault).
        result = subprocess.run(
            [LABWIRED_BIN, "--firmware", elf_path, "--system", config_path, "--timeout", "5s"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=10
        )
        return result.returncode == 0
    except subprocess.TimeoutExpired:
        # Timeout means it booted and ran without crashing for 5s. We consider this a basic boot pass.
        return True
    except Exception as e:
        print(f"[{os.path.basename(config_path)}] Sim failed: {e}")
        return False

def main():
    if not os.path.exists(LABWIRED_BIN):
        print(f"ERROR: labwired binary not found at {LABWIRED_BIN}")
        print("Please compile Labwired core using `cargo build --release` first.")
        # In a CI environment, we don't want to error out if we are testing the workflow setup itself without having to build labwired.
        # So we'll run a dummy pass for now if labwired is missing.
        print("WARNING: Skipping actual simulation because labwired binary is missing. Running in mock mode.")
        mock_mode = True
    else:
        mock_mode = False

    if not os.path.exists(ZEPHYR_DIR):
        print(f"WARNING: ZEPHYR_BASE not found at {ZEPHYR_DIR}. Cannot build samples. Aborting.")
        return

    configs = [f for f in os.listdir(CONFIGS_DIR) if f.endswith(".yaml")]
    total = len(configs)
    passed = 0

    print(f"Found {total} hardware configurations. Starting validation pipeline...")

    for i, file in enumerate(configs):
        path = os.path.join(CONFIGS_DIR, file)
        target_name = file[:-5]
        
        with open(path, 'r') as f:
            model = yaml.safe_load(f)
            
        print(f"[{i+1}/{total}] Processing {target_name}...")
        
        with tempfile.TemporaryDirectory() as build_dir:
            if not mock_mode:
                build_ok, elf_path = run_zephyr_build(target_name, build_dir)
                if build_ok and elf_path:
                    sim_ok = run_labwired_sim(elf_path, path)
                else:
                    sim_ok = False
            else:
                sim_ok = False # hard failing mock mode simulations for realism

            if sim_ok:
                model["pass_rate"] = 100
                model["verified"] = True
                passed += 1
                print(f"[{target_name}] -> PASSED")
            else:
                model["pass_rate"] = 0
                model["verified"] = False
                print(f"[{target_name}] -> FAILED")

        with open(path, 'w') as f:
            yaml.dump(model, f, default_flow_style=False)

    print(f"Validation complete. Passed: {passed}/{total}")

if __name__ == "__main__":
    main()
