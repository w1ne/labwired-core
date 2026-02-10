# Interactive Debugging with LabWired

LabWired supports interactive debugging using the **Debug Adapter Protocol (DAP)**. This allows you to use standard IDEs like **Visual Studio Code** to step through firmware, set breakpoints, and inspect the CPU state.

## 🚀 Quick Start (VS Code)

### 1. Prerequisites
- **LabWired DAP Server**: Build the server using `cargo build -p labwired-dap`.
- **LabWired VS Code Extension**:
  - Navigate to `vscode`.
  - Run `npm install && npm run compile`.
  - Open the `vscode` folder in a new VS Code window and press `F5` to launch the extension.

### 2. Configuration
Create a `.vscode/launch.json` file in your firmware project:

```json
{
    "version": "0.2.0",
    "configurations": [
        {
            "type": "labwired",
            "request": "launch",
            "name": "Debug Firmware",
            "program": "${workspaceFolder}/target/thumbv7m-none-eabi/debug/firmware",
            "stopOnEntry": true
        }
    ]
}
```

### 3. Debugging Features
- **Source-Level Debugging**: If your ELF file contains DWARF debug information, LabWired will automatically map instruction addresses back to your C or Rust source code.
- **Breakpoints**: Set breakpoints directly in your source code.
- **Stepping**: Use the standard Step Over, Step Into, and Continue commands.
- **Register Inspection**: View the current values of CPU registers in the **Variables** view:
  - **ARM**: R0-R15 (including SP, LR, and PC).
  - **RISC-V**: x0-x31 and PC.

## 🛠 Advanced Usage

### Manual DAP Launch
You can run the DAP server manually for non-VS Code integrations or debugging the server itself:
```bash
labwired-dap --log-file debug.log
```
The server communicates via stdin/stdout using the DAP JSON-RPC protocol.

### Symbol Resolution
LabWired uses the `addr2line` and `gimli` crates to resolve symbols. Ensure your firmware is compiled with debug symbols (e.g., `debug = true` in `Cargo.toml` profiles or `-g` in GCC).

## 📝 Troubleshooting
- **No Source Code appearing**: Ensure the `program` path in `launch.json` points to an ELF with debug symbols.
- **"Unknown Instruction" during debug**: This usually means the firmware hit a Thumb-2 instruction or FPU operation not yet implemented in the core engine.
