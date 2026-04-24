# Debugging Firmware

LabWired exposes two debug interfaces — both talk to the same running
simulator, so pick whichever your IDE supports:

- **GDB RSP** on `localhost:3333` — standard GDB, VS Code Cortex-Debug,
  Ozone, any tool that speaks Remote Serial Protocol.
- **DAP** (Debug Adapter Protocol) — used by the LabWired VS Code
  extension directly, no GDB required.

## GDB from the command line

```bash
labwired --firmware path/to/fw.elf --system system.yaml --gdb 3333
```

In another terminal:

```bash
arm-none-eabi-gdb path/to/fw.elf        # or riscv64-unknown-elf-gdb
(gdb) target remote localhost:3333
(gdb) b main
(gdb) c
```

Supported RSP packets: `g`/`G` (all regs), `P` (single reg), `m`/`M`
(memory), `Z0`/`z0` (sw breakpoints), `vCont` (step/continue),
`qSupported`, `Ctrl-C` (break).

## VS Code + Cortex-Debug (ARM)

Install [Cortex-Debug](https://marketplace.visualstudio.com/items?itemName=marus25.cortex-debug),
then in `.vscode/launch.json`:

```json
{
  "version": "0.2.0",
  "configurations": [{
    "type": "cortex-debug",
    "request": "launch",
    "name": "LabWired",
    "servertype": "external",
    "gdbTarget": "localhost:3333",
    "executable": "${workspaceRoot}/target/thumbv7m-none-eabi/debug/firmware",
    "cwd": "${workspaceRoot}",
    "runToEntryPoint": "main",
    "armToolchainPath": "/usr/bin"
  }]
}
```

For RISC-V use the generic C/C++ extension and point it at
`riscv64-unknown-elf-gdb`.

## VS Code + LabWired extension (DAP)

The LabWired VS Code extension ships its own `labwired` debug adapter
and does not require a GDB toolchain. See the extension README for
installation; the launch config is simply:

```json
{
  "type": "labwired",
  "request": "launch",
  "name": "Debug Firmware",
  "program": "${workspaceFolder}/target/thumbv7m-none-eabi/debug/firmware",
  "system": "${workspaceFolder}/system.yaml"
}
```

## What works

Breakpoints, step over/into/out, register view (R0–R15 ARM / x0–x31
RISC-V + PC), memory view across flash / RAM / MMIO, and watch
expressions. Cortex-Debug can render peripheral registers from an SVD
file alongside the simulator.

## Troubleshooting

- **Connection refused** — the simulator didn't start. Check the task
  terminal for an error loading the ELF or the system YAML.
- **GDB binary not found** — install the target toolchain
  (`arm-none-eabi-gdb` / `riscv64-unknown-elf-gdb`) and make sure it's
  on `PATH`.
- **Instruction trace** — add `--trace` when launching `labwired` to
  log every decoded instruction. Noisy but useful for debugging the
  decoder itself, not application code.
