# VS Code Debugging Guide

## Quick Start

1. **Open the demo-blinky folder in VS Code**:
   ```bash
   code examples/demo-blinky
   ```

2. **Install required extensions**:
- LabWired extension (`w1ne.labwired-vscode`)
- Cortex-Debug (`marus25.cortex-debug`)

3. **Build debug firmware**:
   ```bash
   cd ../../
   cargo build -p demo-blinky --target thumbv7m-none-eabi
   ```

## Debug Profiles in `.vscode/launch.json`

The project now has three launch profiles:

1. `LabWired: Demo Blinky`
- Native LabWired DAP path.

2. `Cortex-Debug: LabWired (GDB :3333)`
- Uses Cortex-Debug against LabWired GDB server at `localhost:3333`.

3. `Cortex-Debug: Hardware (OpenOCD ST-Link)`
- Uses Cortex-Debug + OpenOCD on real STM32F103 hardware via ST-Link.

## Same ELF, Two Targets (Recommended Flow)

Use the same binary for both debug targets:
`core/target/thumbv7m-none-eabi/debug/demo-blinky`

### A) LabWired via Cortex-Debug

1. Start LabWired GDB server in terminal:
   ```bash
   ./scripts/start_labwired_gdb.sh
   ```
2. In VS Code Run and Debug, launch:
   `Cortex-Debug: LabWired (GDB :3333)`

### B) Real Hardware via Cortex-Debug

1. Connect STM32F103 board via ST-Link.
2. In VS Code Run and Debug, launch:
   `Cortex-Debug: Hardware (OpenOCD ST-Link)`

## Notes

- OpenOCD config uses:
  - `interface/stlink.cfg`
  - `target/stm32f1x.cfg`
- If your board/debug probe differs, adjust `configFiles` in `.vscode/launch.json`.
- If port `3333` is busy, change both:
  - `core/examples/demo-blinky/scripts/start_labwired_gdb.sh`
  - `Cortex-Debug: LabWired (GDB :3333)` `gdbTarget`

## Troubleshooting

### Cortex-Debug cannot connect to LabWired
- Ensure `start_labwired_gdb.sh` is running.
- Verify port with `ss -ltnp | rg 3333`.

### OpenOCD launch fails
- Ensure OpenOCD is installed.
- Confirm ST-Link USB access and board power.

### Breakpoints not binding
- Rebuild debug binary:
  ```bash
  cargo build -p demo-blinky --target thumbv7m-none-eabi
  ```
