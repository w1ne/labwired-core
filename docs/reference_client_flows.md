# LabWired Reference Client Flows

The [Simulation Protocol](./simulation_protocol.md) provides a deterministic contract for simulation. The true power of LabWired, however, is unlocked when this protocol is integrated into your existing development tools. 

This document outlines the standard, supported "Client Flows" for integrating LabWired into CI pipelines, Interactive IDE Debugging, and AI Agent workflows.

---

## 1. Headless CI Integration (GitHub Actions)

The primary use-case for the Simulation Protocol is deterministic, headless regression testing in Continuous Integration (CI) pipelines.

### The CI Flow
1. **Trigger**: A developer pushes code or opens a Pull Request.
2. **Setup**: The CI runner checks out the code, builds the firmware (e.g., `cargo build` or `make`), and downloads the `labwired-cli`.
3. **Execution**: The runner invokes `labwired test` parsing a predefined `test_script.yaml`.
4. **Assertion**: LabWired executes the simulation deterministically and asserts against the defined limits (cycles, UART output).
5. **Reporting**: LabWired exits with standard POSIX codes and emits strict artifact files (`result.json`, `junit.xml`, `trace.vcd`). 
6. **Integration**: CI systems natively ingest `junit.xml` to display inline success/failure on the Pull Request.

### Reference Configuration (`.github/workflows/sim.yml`)

```yaml
name: Firmware Simulation
on: [push, pull_request]

jobs:
  simulate:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      # Step 1: Build the firmware
      - name: Build Firmware
        run: cargo build --release --target thumbv7em-none-eabihf
        
      # Step 2: Install LabWired
      - name: Install LabWired CLI
        uses: labwired/setup-action@v1
        with:
          version: 'latest'
          
      # Step 3: Run the deterministic protocol headless
      - name: Run Simulation
        run: labwired test --script tests/hardware_validation.yaml
        
      # Step 4: Archive deterministic artifacts (VCD traces) on failure
      - name: Upload Traces
        if: failure()
        uses: actions/upload-artifact@v4
        with:
          name: failure-traces
          path: trace.vcd
```

---

## 2. Interactive IDE Debugging (VS Code)

While the protocol excels at headless execution, developers need interactive introspection. LabWired translates the deterministic protocol into standard GDB-RSP (Remote Serial Protocol) to "trick" standard debuggers into talking to the simulator as if it were a physical J-Link adapter.

### The Interactive Flow
1. **Start Server**: The developer runs `labwired gdbserver --firmware app.elf --system system.yaml`. LabWired initializes the simulation state and pauses, opening a TCP port (e.g., `3333`).
2. **Attach Interface**: The IDE's debugger adapter (e.g., `cortex-debug` in VS Code) connects to TCP port `3333`.
3. **Debug**: The developer sets breakpoints, steps through code, and inspects variables. LabWired translates GDB's `vCont` (step) commands into precise Instruction-Level advances in the simulator core.

### Reference Configuration (`.vscode/launch.json`)

To use LabWired with the popular `cortex-debug` extension in VS Code:

```json
{
  "version": "0.2.0",
  "configurations": [
    {
      "name": "LabWired: Debug STM32",
      "type": "cortex-debug",
      "request": "launch",
      "cwd": "${workspaceRoot}",
      "executable": "${workspaceRoot}/target/thumbv7em-none-eabihf/release/app",
      "servertype": "external",
      "gdbTarget": "localhost:3333",
      "gdbPath": "arm-none-eabi-gdb",
      "preLaunchTask": "Start LabWired GDBSERVER",
      "svdFile": "${workspaceRoot}/chips/stm32f401.svd",
      "runToEntryPoint": "main",
    }
  ]
}
```

---

## 3. Agent Integration (AIPi Toolset)

LabWired acts as a **deterministic hardware oracle** for AI agents. Rather than a purely conceptual framework, LabWired is building a dedicated toolset that external orchestrators can use to safely generate, verify, and emulate hardware peripherals against vendor datasheets.

### The Agentic "Iterative Loop" Protocol
Agents interact with the simulator via an iterative reinforcement loop:
1.  **Hypothesize**: The agent extracts an initial model structure from unstructured data (e.g., a PDF datasheet).
2.  **Simulate**: The agent loads the model into the LabWired sandbox.
3.  **Verify**: The agent applies stimulus (register writes) and checks responses (reads/interrupts).
4.  **Audit**: The simulation behavior is compared against formal `HardwareRules`.
5.  **Fix**: If a deviation occurs, the agent updates the model and repeats the loop.

### Web-Based Agent APIs (Coming Soon)

To ensure the highest fidelity and control, **the complete AIPi toolkit and Agentic execution APIs will be provided exclusively through the LabWired Web Platform in a future update.** 

Users will be able to connect their LLM pipelines or RL fuzzers directly to our managed cloud infrastructure, executing the "Iterative Loop" without needing to manage local Python SDKs or compile Rust toolchains.

*(Note: Enterprise MAESTRO fuzzing integration examples will also be made available to commercial tier subscribers via the web portal).*
