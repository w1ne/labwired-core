# Debugging (DAP)

LabWired implements a native **Debug Adapter Protocol (DAP)** server, enabling direct integration with IDEs like VS Code without intermediate GDB processes.

## 1. Architecture

The DAP server operates as a sidecar process or an internal thread within the simulation runner.

- **Protocol**: JSON-RPC over Standard Input/Output (stdio) or TCP.
- **Capabilities**:
    - `launch`: Starts a new simulation instance.
    - `attach`: Connects to a running simulation.
    - `setBreakpoints`: Supports source-level and instruction-level breakpoints.
    - `threads`: Exposes the CPU core as a single thread.
    - `stackTrace`: Unwinds the stack frame based on the current PC and SP.
    - `scopes` / `variables`: Inspects registers and local variables.

## 2. VS Code Integration

The `labwired-vscode` extension bundles the DAP client.

### Launch Configuration (`launch.json`)

To debug a firmware image, define a generic DAP launch configuration:

```json
{
    "version": "0.2.0",
    "configurations": [
        {
            "type": "labwired",
            "request": "launch",
            "name": "Debug Firmware",
            "program": "${workspaceFolder}/target/thumbv7m-none-eabi/debug/firmware",
            "args": ["--system", "config/system.yaml"],
            "stopOnEntry": true,
            "cwd": "${workspaceFolder}"
        }
    ]
}
```

### Configuration Options
- **program**: Path to the ELF binary with debug symbols (DWARF).
- **args**: Command-line arguments passed to the LabWired CLI.
- **stopOnEntry**: If `true`, the simulator halts at the Reset Vector.
- **cwd**: Current working directory for the simulation process.

## 3. Telemetry Extensions

The LabWired DAP implementation extends the protocol to support real-time telemetry.

### Custom Events
The server emits a `telemetry` event every 100ms containing performance metrics.

**Payload Schema:**
```json
{
  "type": "event",
  "event": "telemetry",
  "body": {
    "cycles": 120500,
    "mips": 12.5,
    "pc": "0x080001a4"
  }
}
```

This allows the VS Code extension to render a live "Dashboard" view without polling the debug interface, minimizing protocol overhead.

## 4. Troubleshooting

### "Unknown Request" Errors
If the debug console shows protocol errors:
1.  Verify the `labwired-cli` version matches the extension version.
2.  Enable verbose logging in `launch.json`: `"trace": true`.

### Source Mapping Issues
If breakpoints cannot be set:
1.  Ensure the ELF was compiled with `debug = true`.
2.  Verify the `program` path matches the binary being displayed in the editor.
