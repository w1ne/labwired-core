# GDB Integration Guide

LabWired features a built-in **GDB Remote Serial Protocol (RSP)** server that allows you to connect professional debugging tools (GDB, Ozone, Cortex-Debug) directly to the simulation.

## 🔗 Connecting to GDB Server

By default, the GDB server starts automatically when you launch a simulation session via the DAP server (e.g., in VS Code).

- **Default Address**: `localhost:3333`
- **Protocol**: GDB Remote Serial Protocol (RSP)

### 🐚 Using Command Line GDB

1.  Start your simulation (e.g., via VS Code extension).
2.  In a terminal, run your cross-compiler GDB:
    ```bash
    arm-none-eabi-gdb target/firmware
    ```
3.  Connect to the LabWired target:
    ```gdb
    (gdb) target remote localhost:3333
    ```
4.  You can now use standard GDB commands:
    - `i r`: Info registers
    - `x/16wx 0x0`: Examine memory
    - `b main`: Set breakpoint
    - `c`: Continue
    - `s`: Step

---

## 💻 Visual Studio Code (Cortex-Debug)

For a superior debugging experience with peripheral views and disassembly, we recommend using the **Cortex-Debug** extension.

### Configuration (`launch.json`)

Add the following to your `.vscode/launch.json`:

```json
{
    "version": "0.2.0",
    "configurations": [
        {
            "type": "cortex-debug",
            "request": "launch",
            "name": "LabWired (Cortex-Debug)",
            "servertype": "external",
            "gdbTarget": "localhost:3333",
            "executable": "${workspaceRoot}/examples/arm-c-hello/target/firmware",
            "cwd": "${workspaceRoot}",
            "runToEntryPoint": "main",
            "armToolchainPath": "/usr/bin"
        }
    ]
}
```

---

## 🧪 Automated Testing

We provide a verification script to test the GDB interface stability:

```bash
python3 scripts/test_gdb_rsp.py
```

This script is also part of our **CI Pipeline**, ensuring that the GDB interface remains robust across all releases.

---

## 🛠️ Supported Packets

LabWired supports the core set of RSP packets required for professional debugging:
- `g`/`G`: Read/Write all registers
- `P`: Write single register
- `m`/`M`: Read/Write memory (hex)
- `Z0`/`z0`: Insert/Remove software breakpoints
- `vCont`: Resumption and stepping
- `qSupported`: Capability negotiation
- `Ctrl-C`: Break/Interrupt simulation
