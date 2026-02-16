# CLI Reference

The `labwired` command-line interface is the primary entry point for running simulations, testing, and managing assets.

## Global Options

These options apply to the interactive runner and most subcommands.

| Option | Description |
| :--- | :--- |
| `--trace` | Enable instruction-level execution tracing (prints every executed instruction). |
| `--json` | Output errors and diagnostics as structured JSON for agent consumption. |
| `--vcd <PATH>` | Output a Value Change Dump (VCD) trace file to the specified path. |
| `--version` | Print version information. |
| `--help` | Print help message. |

## Modes & Commands

### Interactive / Run Mode (Default)
Executes a firmware simulation interactively. If no subcommand is provided, this mode is used.

```bash
labwired [OPTIONS] --firmware <ELF> --system <YAML>
```

**Options:**
- `--firmware <PATH>`: Path to the ELF binary to load (Required).
- `--system <PATH>`: Path to the System Manifest YAML (Required).
- `--max-steps <N>`: Stop simulation after N instructions (default: 20000).
- `--gdb <PORT>`: Start a GDB RSP server on the specified port (e.g., 3333).
- `--breakpoint <ADDR>`: Breakpoint PC address (decimal or 0xHex). Repeatable.
- `--snapshot <PATH>`: Write a state snapshot (JSON) upon exit.

### `test`
Runs a CI-friendly test script with assertions.

```bash
labwired test --script <YAML> [OVERRIDES]
```

**Options:**
- `-c, --script <PATH>`: Path to the test script (see [Test Runner](ci_test_runner.md)).
- `-f, --firmware <PATH>`: Override the firmware path defined in the script.
- `-s, --system <PATH>`: Override the system manifest defined in the script.
- `--output-dir <PATH>`: Directory to write artifacts (`result.json`, `uart.log`).
- `--junit <PATH>`: Path to write JUnit XML report.
- `--max-steps <N>`: Override default step limit.
- `--max-cycles <N>`: Override cycle limit.
- `--max-uart-bytes <N>`: Override UART output limit.
- `--no-progress <N>`: Fail if PC doesn't change for N steps (detects hangs).
- `--no-uart-stdout`: Disable echoing UART output to the console.
- `--max-vcd-bytes <N>`: Limit the size of the generated VCD file.

### `asset`
Utilities for managing LabWired assets (SVD import, Code Generation, etc.).

```bash
labwired asset <SUBCOMMAND>
```

**Subcommands:**
- `import-svd`: Import an SVD file and convert it to Strict IR (JSON).
  - `-i, --input <SVD>`: Input SVD file.
  - `-o, --output <JSON>`: Output JSON file.
- `codegen`: Generate Rust code from Strict IR.
  - `-i, --input <JSON>`: Input IR file.
  - `-o, --output <RS>`: Output Rust source file.
- `init`: Initialize a new project skeleton.
  - `-o, --output <DIR>`: Output directory.
  - `-c, --chip <NAME>`: Chip name to base the project on.
- `add-peripheral`: Add a peripheral to an existing chip descriptor.
  - `-c, --chip <YAML>`: Target chip descriptor.
  - `--id <NAME>`: New peripheral ID.
  - `--base <ADDR>`: Base address.
  - `--ir-path <PATH>`: Path to IR descriptor.
- `validate`: Validate a System Manifest and its referenced Chip.
- `list-chips`: List available chip descriptors.

### `machine`
Machine state control operations.

**Subcommands:**
- `load`: Load a machine state from a snapshot and resume simulation.
  - `-s, --snapshot <JSON>`: Path to snapshot file.
  - `--max-steps <N>`: Override step limit.
  - `--trace`: Enable tracing.

## Environment Variables

- `RUST_LOG`: Controls logging level (e.g., `info`, `debug`).
  - Native logging uses `tracing`.
  - Example: `RUST_LOG=labwired_core=debug`
