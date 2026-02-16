# GDB Integration (RSP)

LabWired implements the GDB Remote Serial Protocol (RSP), allowing standard GDB clients (`arm-none-eabi-gdb`, `gdb-multiarch`) to connect for interactive debugging.

## 1. Server Configuration

The GDB server is embedded within the `labwired-cli`. It listens on TCP port `3333` by default.

### Starting the Server manually
```bash
labwired --gdb --port 3333 --firmware firmware.elf --system system.yaml
```

## 2. Connecting with GDB

Launch your cross-architecture GDB client and connect to the remote target.

```bash
arm-none-eabi-gdb target/firmware.elf
```

Inside GDB:
```gdb
(gdb) target remote localhost:3333
(gdb) load            # Optional: Reloads sections if modified
(gdb) break main
(gdb) continue
```

## 3. Supported Commands

The RSP implementation supports the following subsets:

| Command | Description | Notes |
| :--- | :--- | :--- |
| `g` / `G` | Read/Write All Registers | Full context switch support |
| `P` | Write Single Register | PC modification supported |
| `m` / `M` | Read/Write Memory | Used for variable inspection |
| `Z0` / `z0` | Software Breakpoints | Uses BKPT instruction injection |
| `vCont` | Continue / Step | Supports single-stepping |
| `qSupported` | Feature Negotiation | XML target description |

## 4. IDE Integration (Cortex-Debug)

For VS Code users preferring the GDB workflow (e.g., for extensive peripheral viewing via SVD), configured `launch.json` as follows:

```json
{
    "type": "cortex-debug",
    "request": "launch",
    "servertype": "external",
    "gdbTarget": "localhost:3333",
    "executable": "${workspaceFolder}/target/firmware.elf",
    "runToEntryPoint": "main",
    "svdFile": "${workspaceFolder}/STM32F103.svd"
}
```

**Note**: You must start the LabWired GDB server manually or via a pre-launch task before starting the debug session.
