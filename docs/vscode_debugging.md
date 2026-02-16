# VS Code Debugging Configurations

LabWired supports two primary debugging methods in VS Code: **Native DAP** (Recommended) and **GDB via Cortex-Debug**.

## 1. Native DAP (Recommended)

Uses the LabWired VS Code extension directly. Best for simplicity and live telemetry performance.

**`.vscode/launch.json`**:
```json
{
    "version": "0.2.0",
    "configurations": [
        {
            "type": "labwired",
            "request": "launch",
            "name": "LabWired: Native DAP",
            "program": "${workspaceFolder}/target/thumbv7m-none-eabi/debug/firmware",
            "args": ["--system", "config/system.yaml"],
            "stopOnEntry": true,
            "cwd": "${workspaceFolder}"
        }
    ]
}
```

See [Debugging Guide](debugging.md) for architectural details.

## 2. GDB / Cortex-Debug

Uses the standard `cortex-debug` extension connected to LabWired's GDB server. Best if you need SVD peripheral views or other Cortex-Debug specific features.

**`.vscode/launch.json`**:
```json
{
    "version": "0.2.0",
    "configurations": [
        {
            "type": "cortex-debug",
            "request": "launch",
            "name": "LabWired: GDB Remote",
            "servertype": "external",
            "gdbTarget": "localhost:3333",
            "executable": "${workspaceFolder}/target/thumbv7m-none-eabi/debug/firmware",
            "runToEntryPoint": "main",
            "svdFile": "${workspaceFolder}/STM32F103.svd",
            "cwd": "${workspaceFolder}"
        }
    ]
}
```

**Note**: You must start the LabWired GDB server manually before launching this configuration:
```bash
labwired --gdb --port 3333 --firmware ...
```
