# LabWired Development Guide

This guide covers development workflows for all three components in the LabWired monorepo.

## Repository Structure

```text
labwired/
├── core/          # Rust emulator engine
├── vscode/        # VS Code extension
├── ai/            # Python AI tools
└── docs/          # Platform documentation
```

## Prerequisites

### For Core Emulator
- **Rust** 1.75+ ([Install](https://rustup.rs/))
- **ARM Targets**:
  ```bash
  rustup target add thumbv6m-none-eabi thumbv7m-none-eabi
  ```
- **RISC-V Target**:
  ```bash
  rustup target add riscv32i-unknown-none-elf
  ```

### For VS Code Extension
- **Node.js** 18+ and **npm**
- **VS Code** (for testing)

### For AI Tools
- **Python** 3.10+
- **pip** or **uv**

## Building Components

### Core Emulator
```bash
cd core

# Build all host crates
cargo build --workspace --exclude firmware --exclude firmware-ci-fixture --exclude riscv-ci-fixture

# Run tests
cargo test --workspace --exclude firmware --exclude firmware-ci-fixture --exclude riscv-ci-fixture

# Build firmware examples (requires embedded targets)
cargo build -p firmware --target thumbv7m-none-eabi --release
```

### VS Code Extension
```bash
cd vscode

# Install dependencies
npm install

# Compile TypeScript
npm run compile

# Run extension (opens new VS Code window)
code . && press F5
```

### AI Tools
```bash
cd ai

# Install dependencies
pip install -r requirements.txt

# (Future) Run tools
python scripts/extract_datasheet.py
```

## Testing

### Core
```bash
cd core
cargo test --workspace --exclude firmware --exclude firmware-ci-fixture --exclude riscv-ci-fixture
cargo clippy --workspace --exclude firmware --exclude firmware-ci-fixture --exclude riscv-ci-fixture -- -D warnings
cargo fmt --all -- --check
```

### VS Code Extension
```bash
cd vscode
npm run compile
# Manual testing via F5 in VS Code
```

## CI Integration

The root `.github/workflows/` contains CI configurations that automatically:
- Build and test the core emulator
- Run clippy and formatting checks
- Execute integration tests

All workflows are configured to work with the `core/` subdirectory structure.

## Release Process

See [`core/docs/release_strategy.md`](./core/docs/release_strategy.md) for the full release protocol.

## Contributing

1. Create a feature branch: `git checkout -b feature/my-feature`
2. Make changes in the relevant component directory
3. Run tests locally
4. Submit a PR to `develop`

## Common Tasks

### Running a Simulation
```bash
cd core
cargo run -p labwired-cli -- --firmware path/to/firmware.elf --system system.yaml
```

### Debugging with VS Code Extension
1. Build the core: `cd core && cargo build -p labwired-dap`
2. Build the extension: `cd vscode && npm run compile`
3. Open VS Code and press F5 to launch the extension
4. Use the Debug view to connect to a running simulation

## Troubleshooting

### "Cargo workspace not found"
Make sure you're in the `core/` directory when running cargo commands.

### VS Code Extension Not Loading
Ensure you've run `npm install` and `npm run compile` in the `vscode/` directory.

### Python Import Errors
Install dependencies: `cd ai && pip install -r requirements.txt`
