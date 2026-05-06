# LabWired Platform

> **The Deterministic Hardware Oracle for AI Agents.**
> *Programmable, Metered, and Observable Firmware Simulation.*

## Start Here

If you are evaluating LabWired as a user, use this order:

1. Read [DEVELOPMENT.md](./DEVELOPMENT.md) for setup.
2. Run a local example from [`core/examples/ci/`](./core/examples/ci/).
3. Use [`docs/specs/compatibility_matrix.md`](./docs/specs/compatibility_matrix.md) to understand support boundaries.
4. Treat hosted Foundry and catalog expansion workflows as secondary or beta workflows.

## CI Dashboard

| Indicator | Status | Link |
|---|---|---|
| Core Integration | ![Core Integration](https://github.com/w1ne/labwired/actions/workflows/core-ci.yml/badge.svg?branch=main) | [Workflow](https://github.com/w1ne/labwired/actions/workflows/core-ci.yml) |
| VS Code Integration | ![VS Code Integration](https://github.com/w1ne/labwired/actions/workflows/vscode-ci.yml/badge.svg?branch=main) | [Workflow](https://github.com/w1ne/labwired/actions/workflows/vscode-ci.yml) |
| VS Code Nightly | ![VS Code Nightly](https://github.com/w1ne/labwired/actions/workflows/vscode-nightly.yml/badge.svg?branch=main) | [Workflow](https://github.com/w1ne/labwired/actions/workflows/vscode-nightly.yml) |

Core verification dashboards (coverage, unsupported audit, board model validation, nightly) are owned by `labwired-core`:
[`./core/README.md`](./core/README.md)

Explore the [Documentation Hub](./docs/README.md) for strategy and platform-level guides.

## Ownership Model

- Root `labwired` repo owns model delivery and integration packaging for `labwired-core` consumption.
- `labwired-core` repo owns simulator engine correctness and heavy verification workflows.
- Root CI stays lean for fast merge feedback; heavy validation runs in `labwired-core`.
- Hardware target validation and catalog pass-rate refresh are owned by `labwired-core` (`core-validate-hw-targets.yml`).
- Foundry catalog consumes validation metadata from `core/configs/onboarding/*.yaml` (`validation.run_url` + `validation.artifacts_url`).

## Product Shape

Today, the most launchable LabWired experience is:
- local deterministic firmware simulation
- machine-readable CI artifacts
- VS Code-assisted debugging

Hosted Foundry and catalog expansion remain important, but they are not yet the primary onboarding path.

## 🤖 Agent-First Architecture
LabWired is also built as an **API for Agents (AIPi)**. While it offers human-readable interfaces (VS Code, CLI), its long-term mission is to serve as the **"Remote Hands and Eyes"** for autonomous AI agents verifying hardware.

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
- **Model Consumer**: Executes delivered board/chip models shipped through this monorepo flow.

### [`ai/`](./ai/) - The Agent Toolset (AIPi)
The primary interface for autonomous interaction.
- **Schematic Intelligence**: VLM-based perception of hardware topology.
- **Datasheet Ingestion**: "Chain-of-Thought" grounding for generating peripheral models.
- **Zero-Touch Pipeline**: `auto-ingest` orchestrator with LLM-assisted retries and confidence scoring.
- **Telemetry**: Usage-based metering with automatic Foundry export.

### [`vscode/`](./vscode/) - Human Observer (Debug)
A secondary interface for human verification of agent outputs.
- **Timeline View**: Visual confirmation of agent-driven execution.
- **Register Inspector**: Manual spot-checking of peripheral state.
- **Advanced Debugging**: Conditional breakpoints, data breakpoints, watch expressions, live hovers, and Thumb-2 disassembly.

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
- **📖 [Documentation Hub](./docs/README.md)** - Central index for all platform documentation.
  - [Getting Started Tutorial](./docs/tutorials/getting-started.md)
  - [CI Integration Guide](./docs/tutorials/ci-integration.md)
  - [Compatibility Matrix](./docs/specs/compatibility_matrix.md)
  - [Vision Scoreboard](./docs/strategy/vision/SCOREBOARD.md)
  - [Strategy & Planning](./docs/strategy/plan.md)
  - [Development Guide](./DEVELOPMENT.md)
- **🤖 [Agent Instructions](./AGENTS.md)** - Repository-level instructions for coding agents.
- **⚙️ [Core Emulator](./core/README.md)** - Detailed emulator engine documentation.
- **🔌 [VS Code Extension](./vscode/README.md)** - IDE integration features and usage.

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
We follow **trunk-based development** on `main` and enforce strict quality gates.

- **Main Branch**: `main`.
- **Feature Branches**: `feature/xyz` or `feature/issue-id-description`.
- **Hotfix Branches**: `hotfix/xyz`.

**Quality Gates:**
- Root PRs must pass lean integration gates.
- Core engine gates and coverage thresholds are enforced in `labwired-core` workflows.

See [Release & Merging Strategy](core/docs/release_strategy.md) for the full protocol.

## 📄 Documentation
- [Platform Documentation Hub](./docs/README.md)
- [Implementation Plan](./docs/strategy/plan.md)
- [Core Architecture](./core/docs/architecture.md)
- [Release Strategy](./core/docs/release_strategy.md)


## ⚖️ License
MIT
