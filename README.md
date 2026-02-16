# LabWired Platform

> **The Deterministic Hardware Oracle for AI Agents.**
> *Programmable, Metered, and Observable Firmware Simulation.*

## 🤖 Agent-First Architecture
LabWired is built primarily as an **API for Agents (AIPi)**. While it offers human-readable interfaces (VS Code, CLI), its core mission is to serve as the **"Remote Hands and Eyes"** for autonomous AI agents verifying hardware.

It provides agents with:
1.  **Deterministic Execution**: Bit-accurate simulation that yields identical results every run.
2.  **Usage Telemetry**: Precise `Simulation Minutes` tracking for economic reasoning.
3.  **Structured Observability**: JSON/Strict-IR inputs and outputs, eliminating "screen scraping."

## 🏗️ Monorepo Structure

### [`core/`](./core/) - The Oracle Engine
The immutable source of truth for hardware behavior.
- **Strict IR**: Ingests VLM-extracted netlists and JSON models.
- **Headless by Design**: accurate simulation without UI overhead.
- **CI/Agent Runner**: Deterministic execution for automated pipelines.

### [`ai/`](./ai/) - The Agent Toolset (AIPi)
The primary interface for autonomous interaction.
- **Schematic Intelligence**: VLM-based perception of hardware topology.
- **Datasheet Ingestion**: "Chain-of-Thought" grounding for generating peripheral models.
- **Telemetry**: Usage-based metering for the agent economy.

### [`vscode/`](./vscode/) - Human Observer (Legacy/Debug)
A secondary interface for human verification of agent outputs.
- **Timeline View**: Visual confirmation of agent-driven execution.
- **Register Inspector**: Manual spot-checking of peripheral state.

## 🚀 Quick Start

### Building the Core Emulator
```bash
cd core
cargo build --release
```

### Running a Simulation
```bash
cd core
cargo run -p labwired-cli -- --firmware path/to/firmware.elf --system system.yaml
```

### Development Setup
See [`DEVELOPMENT.md`](./DEVELOPMENT.md) for complete setup instructions for all three components.

## 📚 Documentation & Demos
- **🚀 [Demos & Examples](./DEMOS.md)** - Start here to see LabWired in action.
- **[Core Emulator](./core/README.md)** - Detailed emulator documentation
- **[Agent Instructions](./AGENTS.md)** - Repository-level instructions for coding agents
- **[Board Onboarding Playbook](./core/docs/board_onboarding_playbook.md)** - Config-first workflow for adding new board targets
- **[Postmortems](./docs/postmortems/README.md)** - Incident analysis and prevention records
- **[NUCLEO-H563ZI Example](./core/examples/nucleo-h563zi/README.md)** - Human-run capability showcase (emulator + real board)
- **[NUCLEO-H563ZI Demo Story](./docs/NUCLEO_H563ZI_DEMO.md)** - Marketing/demo narrative and live talk track
- **[VS Code Extension](./vscode/README.md)** - Extension features and usage
- **[AI Tools](./ai/README.md)** - Asset generation tools
- **[Development Guide](./DEVELOPMENT.md)** - Contributing and building
- **[Platform Strategy](./docs/spec/)** - Business roadmaps and market analysis

### CI-Friendly Test Runner (`labwired test`)

Use the deterministic runner mode to drive simulations from a YAML test script and emit machine-readable artifacts:

```bash
cargo build --release -p labwired-cli
./target/release/labwired test --script core/examples/ci/uart-ok.yaml --output-dir out/artifacts --no-uart-stdout
```

See `core/docs/ci_test_runner.md` for schema, exit codes, and artifact formats.

## 🔄 CI Integration

LabWired integrates seamlessly into your CI/CD pipeline, replacing physical hardware with deterministic simulation.

### Quick Start

**GitHub Actions:**
```yaml
- uses: w1ne/labwired/.github/actions/labwired-test@main
  with:
    script: tests/firmware-test.yaml
    output_dir: test-results
```

**GitLab CI:**
```yaml
test:
  script:
    - labwired test --script tests/firmware-test.yaml --output-dir results
  artifacts:
    reports:
      junit: results/junit.xml
```

**Docker (when published):**
```bash
docker run --rm -v $PWD:/workspace ghcr.io/w1ne/labwired:latest \
  test --script tests/firmware-test.yaml
```

### Resources

- **[CI Integration Guide](core/docs/ci_integration.md)** - Complete setup instructions
- **[Workflow Templates](core/examples/workflows/)** - Ready-to-use GitHub Actions & GitLab CI templates
- **[Test Examples](core/examples/ci/)** - Sample test scripts

### Benefits

- ✅ No physical hardware required in CI
- ✅ Deterministic, reproducible results
- ✅ Parallel testing across multiple targets
- ✅ Fast feedback (no flashing delays)
- ✅ Fault injection testing support


## 🤝 Development Workflow
We follow **Gitflow** and enforce strict quality gates.

- **Main Branch**: `main` (Production tags only).
- **Development**: `develop` (Feature integration).
- **Feature Branches**: `feature/xyz`.

**Quality Gates:**
- All PRs must pass CI (Format, Lint, Test, Build).
- Code coverage goal: >80%.

See [Release & Merging Strategy](core/docs/release_strategy.md) for the full protocol.

## 📄 Documentation
- [Platform Docs Index](https://labwired.com/docs.html)
- [Implementation Plan](https://labwired.com/docs/plan.md)
- [Core Architecture](https://labwired.com/docs/explanation/architecture/)
- [Release Strategy](https://labwired.com/docs/development/release_strategy/)
- [CI Integration Guide](https://labwired.com/docs/how-to-guides/ci-cd-integration/)
- [Interactive Debugging](https://labwired.com/docs/explanation/debugging-architecture-dap-gdb/)


## ⚖️ License
MIT
