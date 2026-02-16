# ARM C Hello Example

> Part of the [LabWired Demos](../../../DEMOS.md) suite.

A minimal C example demonstrating LabWired's ARM Cortex-M0 simulation capabilities.

## Building

```bash
make
```

This will compile `src/main.c` and produce `target/firmware` (ELF format).

## Debugging

### Option 1: Built-in LabWired Extension
1. Open this folder in VS Code
2. Press `F5` or select **"LabWired (Built-in DAP)"** from the debug dropdown
3. The debugger will stop at `_start` and you can step through the code

### Option 2: Cortex-Debug (Professional)
1. Install the **Cortex-Debug** extension (recommended automatically)
2. Start the LabWired DAP server first:
   ```bash
   cargo run -p labwired-dap
   ```
   Or use the task: `Ctrl+Shift+P` → "Tasks: Run Task" → "Start LabWired DAP Server"
3. Select **"LabWired (Cortex-Debug)"** from the debug dropdown
4. Press `F5`

The Cortex-Debug configuration provides:
- **Register View**: Live CPU register inspection
- **Memory View**: Hex dump of Flash/RAM
- **Disassembly View**: Instruction-level debugging
- **Peripheral View**: (Future) SVD-based peripheral inspection

## Features Demonstrated

- **Vector Table**: Proper Cortex-M reset handler setup
- **UART Output**: Simple character-by-character UART writes
- **Infinite Loop**: Demonstrates continuous execution with periodic output

## Configuration Files

- `mcu.yaml`: MCU definition (STM32F103-like)
- `system.yaml`: System configuration
- `link.ld`: Linker script for memory layout
- `.vscode/launch.json`: Debug configurations
