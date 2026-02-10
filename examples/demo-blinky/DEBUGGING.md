# VS Code Debugging Guide

## Quick Start

1. **Open the demo-blinky folder in VS Code**:
   ```bash
   code examples/demo-blinky
   ```

2. **Install the LabWired VS Code extension** (if not already installed):
   - Open VS Code Extensions (Ctrl+Shift+X)
   - Search for "LabWired"
   - Click Install

3. **Start Debugging**:
   - Press `F5` or click "Run and Debug"
   - Select "Debug Demo Firmware (Blinky + Sensor)"
   - Firmware will build automatically and launch in debugger

## Features

### Breakpoints
- Click in the gutter next to line numbers to set breakpoints
- Execution will pause when breakpoint is hit
- Inspect variables and registers in the Debug sidebar

### Stepping
- **Step Over** (F10): Execute current line
- **Step Into** (F11): Step into function calls
- **Step Out** (Shift+F11): Step out of current function
- **Continue** (F5): Resume execution

### Register Inspection
- View CPU registers in the "Variables" panel
- Registers update after each step
- Includes: R0-R15, SP, LR, PC, CPSR

## Configuration

The `.vscode/launch.json` file contains two configurations:

1. **Debug Demo Firmware** - Builds and debugs the demo-blinky example
2. **Debug Firmware (Custom)** - For debugging other firmware binaries

### Customizing

Edit `launch.json` to change:
- `program`: Path to your firmware ELF file
- `system`: Path to system configuration YAML
- `stopOnEntry`: Whether to break at entry point

## Troubleshooting

### DAP Server Not Found
Ensure `labwired-dap` is built:
```bash
cargo build -p labwired-dap --release
```

### Breakpoints Not Working
- Ensure firmware is compiled with debug symbols (`debug = true` in Cargo.toml)
- Check that the ELF file path in `launch.json` is correct

### Extension Not Found
The LabWired VS Code extension is located in `vscode/`. Install it:
```bash
cd ../../vscode
npm install
npm run compile
code --install-extension .
```

## Next Steps

For advanced features (peripheral viewers, memory inspector), see the [VS Code Extension Research](../../../docs/vscode_extension_research.md) document.
