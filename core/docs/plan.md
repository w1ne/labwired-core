# LabWired Standalone Simulator - Iteration 1 Plan

## Objective
Deliver a standalone command-line tool (`labwired`) capable of loading an ELF binary and executing a basic simulation loop for an ARM Cortex-M architecture (initially mocked/simplified).

## Roadmap

### 🎯 Release MVP Definition
**Goal**: A professional-grade Firmware Simulation Platform for ARM Cortex-M, integrated into VS Code.

**Core Deliverables**:
1.  **Emulator Core**: Robust, deterministic simulation of STM32F103 (Cortex-M3) with UART/GPIO/Timers.
2.  **VS Code Extension**: "Ozone-like" debugging experience (Timeline, Registers, Memory) connecting via DAP.
3.  **Documentation**: comprehensive guides for "Getting Started" and "VS Code Debugging".

*Note: AI Peripheral Generation is a parallel research track and is NOT required for the v1.0 MVP.*

### 🛑 Anti-Goals (Out of Scope for v1.0)
To ensure we hit our release window, the following are explicitly **excluded**:
*   **Multicore Debugging**: We focus solely on single-core Cortex-M.
*   **Reverse Debugging (Time Travel)**: Deferred to v1.1+.
*   **Custom Peripheral UI**: VS Code will only render standard SVD-based views.

### 🧪 Market Verification
*   **Emulator**: Must run unmodified "Hello World" binaries from standard `stm32-rs` examples.
*   **Extension**: The dev team must dogfood the extension for all internal debugging tasks for 1 week.

### Phase 1: Foundation (Completed)
- [x] Project Structure (Workspace)
    - **Verified**: `Cargo.toml` workspace defines `core`, `loader`, `cli`.
- [x] Release & Merging Strategy Defined (`docs/release_strategy.md`)
    - **Verified**: Document exists and team follows Gitflow.
- [x] Core Traits (CPU, MemoryBus, Peripheral)
    - **Verified**: `crates/core/src/lib.rs` defines `Cpu` and `Bus` traits.
- [x] Error Handling Strategy (`thiserror`)
    - **Verified**: `SimulationError` enum implemented in `crates/core`.
- [x] Logging/Tracing Setup
    - **Verified**: `cli` initializes `tracing_subscriber`, logs visible in stdout.

### Phase 2: Loader (Completed)
- [x] Integrate `goblin` dependency
    - **Verified**: `crates/loader/Cargo.toml` includes `goblin`.
- [x] Implement `ElfLoader` struct
    - **Verified**: `crates/loader/src/lib.rs` implements `load_elf`.
- [x] Parse entry point and memory segments from ELF
    - **Verified**: `ProgramImage` struct successfully populated in `loader` tests.

### Phase 3: Core Simulation Loop (Completed)
- [x] Implement `Cpu` struct (Cortex-M Stub)
    - **Verified**: `CortexM` struct in `crates/core/src/cpu/mod.rs`.
- [x] Implement `Memory` struct (Flat byte array)
    - **Verified**: `LinearMemory` in `crates/core/src/memory/mod.rs`.
- [x] Implement `Bus` to route traffic between CPU and Memory
    - **Verified**: `SystemBus` routes addresses 0x0... to Flash and 0x2... to RAM.
- [x] Basic FE (Fetch-Execute) cycle loop
    - **Verified**: `Machine::step()` fetches instruction from PC and increments it.

### Phase 4: CLI & Basic Decoding (Completed)
- [x] Argument parsing (`clap`)
    - **Verified**: `labwired --help` works, accepts `-f` argument.
- [x] Connect `loader` output to `core` initialization
    - **Verified**: `cli` correctly passes loaded `ProgramImage` to `Machine::load_firmware`.
- [x] Run the simulation loop
    - **Verified**: CLI runs 10 steps of simulation and prints PC updates.
- [x] Implement basic Thumb-2 Decoder (`MOV`, `B`)
    - **Verified**: `crates/core/src/decoder.rs` correctly decodes opcodes `0x202A` (MOV) and `0xE002` (B).
- [x] Verify verification with `tests/fixtures/uart-ok-thumbv7m.elf`
    - **Verified**: Real ELF file loaded and executed in `cli` (see `tests/fixtures/uart-ok-thumbv7m.elf`).

### Phase 5: Verification (Completed)
- [x] Integration tests using a dummy ELF (or just a binary file)
    - **Verified**: `crates/core/src/tests.rs` validates CPU logic.
- [x] CI pipeline
    - **Verified**: GitHub Actions (`ci.yml`) builds and tests on push.

### Phase 6: Infrastructure Portability (Completed)
- [x] Dockerfile for testing
    - **Verified**: `Dockerfile` builds `rust:latest` image.
- [x] Docker-based verification
    - **Verified**: `docker run` successfully executes `cargo test` suite (9/9 passed).

## Iteration 2: Expanded Capabilities (Completed)
- [x] Arithmetic & Logic Instructions
    - **Verified**: `ADD`, `SUB`, `CMP`, `AND`, `ORR`, `EOR`, `MVN` implemented and tested.
- [x] Memory Operations
    - **Verified**: `LDR` and `STR` implemented and verified via integration tests.
- [x] Portable Core Architecture
    - **Verified**: `Machine` is generic over `Cpu` trait.
- [x] UART Peripheral
    - **Verified**: Mapped to `0x4000_C000`, writes to stdout.

## Iteration 3: Firmware Support (Completed)
- [x] Implement Stack Operations
    - **Verified**: `PUSH`, `POP` implemented and tested.
- [x] Implement Control Flow
    - **Verified**: `BL`, `BX` and `Bcc` implemented.
- [x] Implement PC-Relative Load
    - **Verified**: `LDR` (Literal) handles constant pools.
- [x] Firmware Project
    - **Verified**: `crates/firmware` builds and runs via correctly configured `link.x`.
- [x] End-to-End Verification
    - **Verified**: Firmware boots and executes in simulator.

## Iteration 4: Advanced Core Support (Completed)
- [x] Implement High Register Operations
    - **Verified**: `MOV` and `CMP` support R8-R15 (including SP, LR, PC).
- [x] Implement Byte-level Memory Access
    - **Verified**: `LDRB`, `STRB` implemented for buffer manipulation.
- [x] Refine 32-bit Instruction Handling
    - **Verified**: Robust 32-bit reassembly for `BL`, `MOVW`, `MOVT`.
- [x] Milestone: "Hello, LabWired!" achieved via UART peripheral.

## Iteration 5: System Services & Exception Handling (Completed)
- [x] Implement Vector Table Boot Logic
    - **Verified**: CPU automatically loads SP/PC from 0x0 on reset.
- [x] Implement SysTick Timer
    - **Verified**: Standard `SYST_*` registers implemented and ticking.
- [x] Implement Basic Exception Entry/Exit
    - **Verified**: Stacking/Unstacking logic allows interrupt handling.

## Iteration 6: Descriptor-Based Configuration (Completed)
- [x] Implement YAML Chip Descriptors
    - **Verified**: `configs/chips/stm32f103.yaml` defines memory mapping and peripherals.
- [x] Implement System Manifests
    - **Verified**: `system.yaml` allows wiring of sensors and devices.
- [x] Dynamic SystemBus
    - **Verified**: Bus auto-configures based on descriptor files.
- [x] Functional Device Stubbing
    - **Verified**: `StubPeripheral` allows modeling external hardware.

## Iteration 7: Stack & Advanced Flow Control (Completed)
- [x] Implement `ADD SP, #imm` and `SUB SP, #imm`.
    - **Verified**: `AddSp`/`SubSp` variants in `decoder.rs` (lines 26-27), execution in `cpu/mod.rs` (lines 248-254), tested in `test_iteration_7_instructions` (lines 401-410).
- [x] Implement `ADD (High Register)` for arbitrary register addition.
    - **Verified**: `AddRegHigh` variant in `decoder.rs` (line 28), execution in `cpu/mod.rs` (line 256), tested with R0+R8 addition (lines 412-418).
- [x] Implement `CPSIE/CPSID` for interrupt enable/disable control.
    - **Verified**: `Cpsie`/`Cpsid` variants in `decoder.rs` (lines 29-30), execution in `cpu/mod.rs` (lines 305-312), tested with primask flag verification (lines 420-431).
- [x] Milestone: Full execution of standard `cortex-m-rt` initialization without warnings.
    - **Verified**: Test suite passes (33/33 tests), no unknown instruction warnings during execution.

## Iteration 8: Real-World Compatibility (Completed)
- [x] Implement Block Memory Operations (`LDM/STM`)
    - **Verified**: `Ldm`/`Stm` variants in `decoder.rs` (lines 45-46), execution in `cpu/mod.rs` (lines 374-397), tested in `test_iteration_8_instructions` with register list {R0-R2} (lines 446-467).
- [x] Implement Halfword Access (`LDRH/STRH`)
    - **Verified**: `LdrhImm`/`StrhImm` variants in `decoder.rs` (lines 47-48), `read_u16`/`write_u16` in `bus/mod.rs`, execution in `cpu/mod.rs` (lines 250-287), tested with 16-bit memory operations (lines 436-445).
- [x] Implement Multiplication (`MUL`)
    - **Verified**: `Mul` variant in `decoder.rs` (line 49), execution in `cpu/mod.rs` (lines 439-457) with N/Z flag updates, tested with 100×2=200 (lines 468-477).
- [x] Implement NVIC (Nested Vectored Interrupt Controller)
    - **Verified**: `nvic.rs` peripheral created (96 lines), ISER/ICER/ISPR/ICPR registers with atomic state, integrated in `SystemBus::tick_peripherals` (lines 158-198), tested in `test_nvic_external_interrupt` (lines 483-512).
- [x] Implement SCB with VTOR (Vector Table relocation)
    - **Verified**: `scb.rs` peripheral created (42 lines), VTOR register at 0xE000ED08, shared atomic state with CPU, exception handler lookup uses VTOR offset (cpu/mod.rs lines 175-180), tested in `test_vtor_relocation` (lines 514-537).
- [x] Two-phase interrupt architecture with NVIC filtering
    - **Verified**: `SystemBus::tick_peripherals` implements pend→signal flow, external IRQs (≥16) filtered by NVIC ISER/ISPR, core exceptions (<16) bypass NVIC (bus/mod.rs lines 158-198).
- [x] Milestone: All 33 tests passing, v0.6.0 released
    - **Verified**: `cargo test` shows 33/33 passing, release tag v0.6.0 created and pushed to GitHub, CHANGELOG.md updated with all features.

## Iteration 9: Real Firmware Integration & Peripheral Ecosystem (In Progress)

### Objectives
Bridge the "peripheral modeling bottleneck" by enabling execution of real-world HAL libraries and expanding the peripheral ecosystem.

#### Milestone 9.5: Documentation & v0.7.0 Release
- [x] Integrate core peripherals into standard SystemBus
- [x] Document v0.7.0 features in CHANGELOG and README
- [x] Resolve clippy lints and formatting across workspace

**Acceptance Tests**
- `cargo test` and `cargo clippy` pass.
- Release tag `v0.7.0` created.

## Iteration 10: Advanced Debugging & Modular Observability
**Objective**: Transition from "execution capable" to "debug ready" while enforcing a **strictly modular architecture**. Decouple introspection tools from the core execution engine.

### Phase A: Modular Metrics & Performance
- [x] **Decoupled Metric Collectors**: Implement a trait-based system for gathering execution stats.
    - **Verified**: `SimulationObserver` in `crates/core/src/lib.rs` and `PerformanceMetrics` in `crates/core/src/metrics.rs` (released in `CHANGELOG.md` v0.8.0).
- [x] Execution statistics (IPS, instruction count, total cycles)
    - **Verified**: `PerformanceMetrics::{get_instructions,get_cycles,get_ips}` in `crates/core/src/metrics.rs`.
- [x] Real-time IPS display in CLI
    - **Verified**: Periodic IPS logging in `crates/cli/src/main.rs` gated by `--trace` (v0.8.0).
- [x] Per-peripheral cycle accounting (modular ticking costs)

### Phase B: Advanced ISA & Peripheral Expansion
- [x] Bit field instructions (`BFI`, `BFC`, `SBFX`, `UBFX`)
- [x] Misc Thumb-2 instructions (`CLZ`, `RBIT`, `REV`, `REV16`)
- [x] **ADC Peripheral**: Implement as a modular, standalone component.

### Phase C: Pluggable Observability Tools
- [x] **State Snapshots**: Modular format (JSON/YAML) for dumping CPU/Peripheral state.
- [x] **Trace Hooks**: Implement a "subscriber" pattern for memory access and register changes.
- [x] Basic breakpoint support (PC-based halt)

### Phase D: Ecosystem & Documentation
- [x] **Peripheral Development Tutorial**: Guide on creating decoupled, custom sensor mocks.
- [x] Example: STM32 I2C sensor interaction walkthrough.
- [x] **Declarative Register Maps**: Formalize YAML specifications to decouple register logic from Rust code.
- [ ] Documentation: "Getting Started with Real Firmware" guide.

### Success Criteria
- [ ] **Architectural Purity**: Core simulator loop remains unaware of metrics/tracing implementations.
- [ ] Accurate IPS reporting during simulation.
- [ ] Ability to dump full state to external files without stopping simulation.
- [ ] Successful execution of ADC-based HAL examples.

## Strategic Roadmap (Business-Aligned)

This section translates the business research roadmap (“The Strategic Horizon of Firmware Simulation…”) into an executable engineering plan for LabWired. It starts at the product milestone level and decomposes down to implementation tasks.

### Milestone Overview (High-Level)

| Business iteration | Primary outcome | Main artifact | Notes / mapping to this repo plan |
| :--- | :--- | :--- | :--- |
| **1** | Standalone local runner | CLI runner | Largely covered by Iterations 1–8 in this file. |
| **2** | CI-native execution | Test scripting + Docker + GitHub Action | Implemented in Iteration 11 (v0.9.0). |
| **3** | IDE-grade debugging | DAP server + VS Code extension | DAP implemented; GDB support (RSP) planned for later. |
| **4** | Automated peripheral modeling | Model IR + ingestion + verified codegen + registry | Planned as Iteration 13. |
| **5** | Enterprise-scale fleets + compliance | Orchestrator + dashboard + reporting | Planned as Iteration 14. |

### Cross-Cutting Workstreams (Always-On)

**Release Engineering & Quality**
- [ ] Enforce CI quality gates: `cargo fmt -- --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo audit`, `cargo build` (see `docs/release_strategy.md`).
- [ ] Maintain a per-release checklist: version bump, changelog entry, artifacts, docs update, demo verification.
- [ ] Maintain a compatibility matrix (supported MCUs / boards / peripherals / known gaps).

**Determinism & Correctness**
- [ ] Provide deterministic execution controls (stable time base, bounded nondeterminism, reproducible scheduling).
- [ ] Maintain a “golden reference” suite: periodic validation against physical boards for key behaviors.
- [ ] Add regression fixtures per peripheral (reset values, side effects, IRQ behavior).

**Security & Isolation (cloud-facing)**
- [ ] Treat firmware as untrusted input: strict resource limits (CPU time, memory), crash containment, safe defaults.
- [ ] Produce a threat model + mitigations before any multi-tenant execution (Iteration 14).

**Observability**
- [ ] Standardize run artifacts: logs, traces, configs, firmware hash, model versions, results summary.
- [ ] Provide structured exports suitable for attaching to bugs and CI artifacts.

**Market Validation & Adoption**
- [ ] Define initial ICP + wedge use case (e.g., “run STM32 HAL firmware in CI without dev kits”).
- [ ] Create a public demo + tutorial for the wedge use case (product-led growth).
- [ ] Define the open-core boundary (what is OSS vs proprietary) and document the rationale.
- [ ] Establish contribution guidelines for peripherals/models (review process, versioning, compatibility policy).

**Economics & Compliance**
- [ ] Define pricing metrics early (seats vs minutes vs storage) and instrument COGS per run.
- [ ] Start an “enterprise readiness” checklist ahead of Iteration 14 (RBAC, audit logs, retention, SOC2 plan, ISO 26262 evidence scope).

## Iteration 11: Headless CI Integration & Test Runner (Business Iteration 2)
**Objective**: Make simulation a deterministic, scriptable CI primitive with machine-readable outputs and drop-in workflows for GitHub/GitLab.

### Status
- Implemented in `v0.9.0` (see `CHANGELOG.md`) with `labwired test`, a versioned YAML test script schema, JSON/JUnit outputs, and a composite GitHub Action wrapper.

### Phase A: Test Script Specification (YAML)
- [x] Define a stable test schema (YAML recommended):
  - [x] Inputs: firmware path + optional system config.
  - [x] Limits: max steps/cycles, wall-clock timeout, max UART bytes, “no progress” detection.
  - [x] Assertions: UART contains/regex, expected stop reason.
  - [ ] Optional actions: inject UART RX, toggle GPIO, trigger IRQ at time T.
- [x] Implement schema validation with actionable error messages.
- [x] Add a version field (`schema_version`) and compatibility policy (v1.0; legacy v1 supported but deprecated).

### Phase B: Headless Runner Semantics
- [x] Add a dedicated runner mode/subcommand (`labwired test --script <yaml>`).
- [x] Implement deterministic stop conditions (assertions + timeouts + “no progress”/hang detection).
- [x] Standardize exit codes (`0` pass, `1` assertion failure, `2` infra/config error, `3` simulation/runtime error).
- [x] Ensure a run is reproducible from artifacts (firmware hash + system + script + resolved limits).

### Phase C: Reporting for CI Systems
- [x] Emit a JSON summary (`result.json`) with pass/fail, stop reason details, limits, firmware hash, and assertions.
- [x] Emit JUnit XML (`junit.xml`) for CI test reporting.
- [x] Emit an artifact bundle (`result.json`, `uart.log`, `junit.xml`) via `--output-dir`.
- [x] Make UART output capturable as a first-class artifact (stdout streaming remains optional).

### Phase D: Distribution & Automation
- [x] Publish a minimal Docker image for CI use (non-root runtime).
- [x] Define a multi-arch build plan (x86_64 + ARM64) where feasible.
- [x] Create a GitHub Action wrapper (composite action in `.github/actions/labwired-test`).
- [x] Provide ready-to-copy workflows for GitHub Actions and GitLab CI.

### Phase E: Adoption (CI Wedge)
- [x] Add a small catalog of CI-ready examples (one pass + one fail) and document them.
- [x] Publish “hardware-in-the-loop replacement” reference workflows (with caching + artifact upload).

### Success Criteria
- [x] Users can run the same test locally and in CI and get identical outcomes (pass/fail + logs + JSON summary).
- [x] GitHub Action runs a sample script and publishes artifacts on both success and failure.

## Iteration 12: Interactive Debugging (DAP) (Completed)
**Objective**: Provide IDE-grade debugging (breakpoints/step/inspect) via the Debug Adapter Protocol.

- [x] DAP Server Core (Initialize, Launch, Disconnect)
- [x] Machine Debug Control (Breakpoints, stepping)
- [x] VS Code Extension for LabWired
- [x] Register & Variable inspection (Completed)

## Iteration 13: GDB Support (Remote Serial Protocol) (In Progress)
**Objective**: Allow GDB to connect to the simulation for command-line debugging and integration with other IDEs.

- [x] Implement GDB RSP server using `gdbstub`.
- [x] Add `--gdb` flag to LabWired CLI.
- [x] Support register and memory access via GDB (ARM & RISC-V).

## Iteration 13.5: Multi-Architecture Foundation (Completed)
**Objective**: Decouple the core simulation engine from Cortex-M specifics to support future architectures (e.g., RISC-V).

- [x] **Generic CPU Trait**: Refined `Cpu` trait to be fully architecture-agnostic.
- [x] **Separated Cortex-M**: Moved `CortexM` implementation to `crates/core/src/cpu/cortex_m.rs`.
- [x] **Decoupled Peripherals**: Removed architecture-specific interrupt logic (EXTI) from `SystemBus`.
- [x] **Generic Interrupts**: Implemented `explicit_irqs` in `PeripheralTickResult` for direct NVIC signaling.
- [x] **System Config**: Created `configure_cortex_m` helper for standardized system setup.

## Iteration 14: Asset Foundry (AI Modeling)
**Objective**: Break the peripheral modeling bottleneck by introducing a validated, versioned model pipeline (SVD/PDF → IR → verified codegen → registry).

### Phase A: Model Intermediate Representation (IR)
- [x] Define a strict IR for peripherals (Rust Structs + Serde):
  - [x] Registers, fields, reset values, access types.
  - [x] **Standardized Side Effects**: `WriteOneToClear`, `ReadToClear`.
  - [ ] **Timing Hooks**: Signal propagation delay, clock domain crossing simulation.
- [ ] Define a compatibility policy (required behaviors vs "best-effort" approximations).

### Phase B: Ingestion (SVD + PDF)
- [ ] **Advanced SVD Parsing**:
  - [ ] Flatten `RegisterClusters` (arrays of structs).
  - [ ] Unroll Register Arrays (`dim` / `dimIncrement`).
  - [ ] Resolve `derivedFrom` inheritance strictly.
- [ ] **Datasheet/PDF Ingestion Pipeline**:
  - [ ] Extract register tables and strictly map to SVD offsets.
  - [ ] Extract timing diagrams and protocol constraints.
  - [ ] Chunk text by peripheral section for RAG context windowing.

### Phase C: AI Synthesis (RAG Agent)
- [ ] **Prompt Engineering**:
  - [ ] Design prompts to output structured `Configuration` or `SystemRDL` (not raw code).
  - [ ] "Chain of Thought" validation: Ask LLM to explain *why* it thinks a bit is "Write-1-to-Clear".
- [ ] **Behavioral Extraction**:
  - [ ] Identify interrupt triggers (e.g., "TXE flag set when TDR is empty").
  - [ ] Identify state machine transitions (e.g., "Enable bit starts the counter").

### Phase D: Verification & Code Generation
- [ ] **SystemRDL Generation**:
  - [ ] Emit standardized SystemRDL 2.0 files as the "Golden Source" of truth.
  - [ ] Validate RDL against known checker tools.
- [ ] **Rust Codegen**:
  - [ ] Implement `SystemRDL -> Rust Peripheral` compiler.
  - [ ] Generate `bitflags` structs for all registers.
  - [ ] Generate default `reset()` and `read()`/`write()` dispatch logic.

### Phase E: Model Registry & Distribution
- [ ] **Artifact Signing**: Sign models with a trusted key to allow "Verified by LabWired" badges.
- [ ] **Versioning**: Semantic versioning for models (e.g., `stm32-usart v1.2.0`).
- [ ] **Community Hub**: CLI command `labwired install <model>`.

## Iteration 15: Enterprise Fleet Management (Business Iteration 5)
**Objective**: Deliver multi-tenant, large-scale parallel simulation with fleet observability, metering, and compliance-oriented reporting.

### Phase A: Product & Tenancy Model
- [ ] Define tenancy hierarchy (Organization → Project → Run).
- [ ] Implement RBAC (Role-Based Access Control): Admin, Editor, Viewer.
- [ ] **Metering engine**: Track "Simulation Minutes" and "Storage Used" for billing.

### Phase B: Orchestration & Isolation (Cloud Native)
- [ ] **Runner Containerization**: Optimize `sim-runner` Docker image (<50MB).
- [ ] **Job Scheduler**:
  - [ ] Priority queues (Enterprise vs Free).
  - [ ] Concurrency limits per organization.
- [ ] **Hardware Isolation**:
  - [ ] **AWS Graviton (ARM64)** optimization for native execution speed (no binary translation).
  - [ ] **Firecracker MicroVMs**: Isolate every run in a disposable VM for security.

### Phase C: Compliance & Reporting (ISO 26262)
- [ ] **Fault Injection Framework**:
  - [ ] API to inject hardware faults (e.g., `gpio.short_to_ground()`, `flash.ecc_error()`).
  - [ ] Campaign runner: "Run this test suite against these 50 fault scenarios".
- [ ] **Evidence Generation**:
  - [ ] Generate immutable execution reports (PDF/JSON) with cryptographic signatures.
  - [ ] **Tool Qualification Kit (TQK)**: Documentation suite verifying the simulator's correctness.
  - [ ] Requirement Traceability: Map test results back to requirements IDs.

### Phase D: Enterprise Dashboard
- [ ] **Live Telemetry**: WebSocket stream of UART/Logs from running jobs.
- [ ] **Snapshot Sharing**: "Copy Link to State" button (stores full RAM/Reg dump).
- [ ] **SSO Integration**: SAML/OIDC for corporate login.

## Iteration 16: VS Code Simulator Management
**Objective**: Enable developers to create, manage, and connect to LabWired simulator instances directly within VS Code, streamlining the workflow for both local and remote development.

### Phase A: Simulator Management UI
- [ ] **Creation Wizard**: GUI to select Architecture, Chip, and Firmware ELF.
- [ ] **Instance List**: View running simulator instances (PID, Port, Status).
- [ ] **Process Control**: Start, Stop, and Restart simulator instances from VS Code.

### Phase B: Connection & Interaction
- [ ] **Automatic Connection**: One-click connect to local instances.
- [ ] **Remote Connection**: Support for connecting to remote/Dockerized instances via TCP/IP.
- [ ] **Output Integration**: Stream simulator stdout/stderr to VS Code Output Channel.
- [ ] **Terminal Integration**: Integrated terminal for interacting with the simulator CLI.

### Success Criteria
- [ ] Users can launch a new simulation from a VS Code command/button.
- [ ] Users can see a list of active simulations and terminate them.
- [ ] Output from the simulator is visible in VS Code.

## Iteration 18: Full-Lifecycle IDE Demonstrator
**Objective**: Provide a "Zero-to-Run" experience where the user can compile firmware and launch a matched simulator instance with a single click.

### Phase A: Build System Integration
- [ ] **Build Task Detection**: Automatically detect `Cargo.toml`, `Makefile`, or `CMakeLists.txt`.
- [ ] **One-Click Compile**: VS Code command to trigger the build and capture the resulting ELF.
- [ ] **ELF-to-Simulator Mapping**: Intelligently suggest a chip/system config based on the build target architecture.

### Phase B: Automated Simulator Provisioning
- [ ] **Auto-Launch**: Automatically start `labwired` with the correct `--firmware` and `--system` flags after a successful build.
- [ ] **Plugin-Driven Lifecycle**: The plugin manages the simulator process as a background task, handling port allocation and cleanup.

### Phase C: Visual Feedback & Control
- [ ] **Simulation Dashboard**: Real-time view of CPU metrics (IPS, PC) and UART logs.
- [ ] **Interactive Control**: Pause/Resume/Reset buttons directly in the IDE.

### Success Criteria
- [ ] A user can open a supported firmware project, click "Run in LabWired", and see simulated UART output without manual CLI interaction.
- [ ] Compilation errors are reported in the VS Code Problems tab.
- [ ] The simulator instance correctly lifecycle-matches the VS Code session.

## Iteration 17: Partner Ecosystem & Growth (Business Strategy)
**Objective**: Leverage the "Partner Programs" of major chip vendors and establish LabWired as a standard industry tool.

### Phase A: Accreditation & Partnerships
- [ ] **ARM Approved Design Partner**: Apply for accreditation to validate technical rigor.
- [ ] **Vendor Sponsorships**: Pitch to ST/NXP/Nordic to sponsor "Verified Models" for their new chips.
- [ ] **Open Source Strategy**: Clearly define the boundary between OSS Core (Runner) and Proprietary (AI Foundry/Cloud).

### Phase B: Community Growth (PLG)
- [ ] **Viral Features**: "Share Snapshot" link generator for StackOverflow/Reddit support capability.

## Iteration 18: Professional Debugger Extension (Ozone-Class UX)
**Objective**: Transform the VS Code extension into a professional-grade debugger matching Segger Ozone's capabilities, with simulation-specific advantages.

**Rationale**: By providing an "Ozone-like" debugging experience natively in VS Code, LabWired becomes the preferred tool for embedded developers, eliminating the need for expensive commercial debuggers for simulation workflows.

### Phase 1: Foundation & Connection (Sprint 1)
**Goal**: establishing a reliable connection between VS Code and the LabWired DAP server.
*   **Deliverable**:
    *   [ ] Connect/Disconnect stable at 100% reliability.
    *   [ ] Standard "Variables" and "Call Stack" views working.
*   **Market Verification (Dogfooding)**:
    *   [ ] Dev team uses this build to debug the *next* feature implementation.

### Phase 2: Visualization (Sprint 2)
**Goal**: The "Ozone-Killer" feature - Timeline View.
*   **Deliverable**:
    *   [ ] Timeline renders PC history for the last 10k cycles.
    *   [ ] Click-to-navigate in timeline.
*   **Market Verification**:
    *   [ ] A user can diagnose a hard-fault loop using *only* the Timeline View in < 1 minute.

### Phase 3: Inspection (Sprint 3)
**Goal**: Deep hardware inspection.
*   **Deliverable**:
    *   [ ] Peripheral Register View (SVD-based).
    *   [ ] Memory View with Hex/ASCII.


### Phase 2: Enhanced Inspection (Weeks 3-4)
- [ ] **Enhanced Register Window**
  - Peripheral registers from YAML configs (UART, GPIO, SPI, etc.)
  - Bit-field expansion (e.g., UART_SR: TXE, RXNE, TC)
  - Change highlighting for recently modified values
  - Custom expression evaluation
- [ ] **Enhanced Memory Window**
  - Hex editor with ASCII preview
  - Editable memory cells (write to simulation)
  - Address bookmarks (Stack, Heap, Peripherals)
  - Data interpretation (uint8/16/32, float, struct overlay)
- [ ] **Instruction Trace Window**
  - Last 100K instructions grouped by call frames
  - Synchronized with source code viewer
  - Timing information per instruction/block

### Phase 3: Advanced Features (Weeks 5-6)
- [ ] **Reverse Debugging (Time Travel)**
  - "Step Back" button implementation
  - Checkpoint/restore mechanism (every 10K instructions)
  - Timeline scrubbing (click to jump to any cycle)
  - Full CPU/memory state restoration
- [ ] **Disassembly Window**
  - Mixed source/assembly view
  - Instruction-level stepping
  - Jump-to-address navigation
  - Cycle count annotations
- [ ] **Code Profiling**
  - Function-level profiling (execution time, call count)
  - Code coverage indicators in source view
  - Hot spot identification
  - Flame graph visualization

### Phase 4: RTOS & Visualization (Weeks 7-8)
- [ ] **RTOS Awareness (FreeRTOS)**
  - Task list with state (running, ready, blocked)
  - Task-specific call stacks
  - Task switch timeline integration
  - Semaphore/mutex/queue inspection
- [ ] **SmartView (Data Visualization)**
  - UART terminal (TX/RX streams)
  - SPI/I2C transaction viewer
  - GPIO pin visualizer
  - Plugin system for custom visualizations

### Phase 5: Multi-Architecture (Weeks 9-10)
- [ ] **Architecture Abstraction Layer**
  - Generic register interface
  - Architecture-specific disassemblers (Capstone integration)
- [ ] **RISC-V Support**
  - Register window (x0-x31, PC, CSRs)
  - RISC-V disassembly
  - Timeline integration
- [ ] **Documentation & Examples**
  - User guide for all debugging features
  - Video tutorials
  - Example projects showcasing advanced features

### Success Criteria
- [ ] Feature parity with Segger Ozone's core debugging capabilities
- [ ] Timeline renders 1M+ instructions smoothly (<100ms)
- [ ] Reverse debugging works reliably (restore state within 500ms)
- [ ] Multi-architecture support (ARM + RISC-V) with consistent UX
- [ ] Positive user feedback from 100+ beta testers

**Strategic Value**: This iteration positions LabWired as a complete professional debugging platform, not just a simulator. It creates a strong moat against competitors and enables premium pricing for enterprise customers who require advanced debugging workflows.

## Strategic Horizon: Future Improvements

These items represent the long-term vision for LabWired, designed to drive significant company revenue through market expansion, enterprise compliance, and technical differentiation.

### 1. Enterprise Safety: ISO 26262 Tool Qualification (Phase III Revenue)
**Objective**: Unlock high-margin automotive and medical contracts through regulatory compliance.
- [ ] **Tool Qualification Kit (TQK)**: Automated validation suites and safety documentation.
- [ ] **ASIL-D Conformance**: Architectural hardening for safety-critical firmware verification.
- [ ] **Traceability Engine**: Linking simulation results directly to requirement IDs.

### 2. Technical Superiority: High-Performance Co-Simulation
**Objective**: Capture the high-complexity SoC and NPU verification market.
- [ ] **Zero-Copy Shared Memory IPC**: <100ns latency bridge for Verilated RTL models.
- [ ] **Edge AI (NPU) Emulation**: Bit-exact modeling of Arm Ethos-U85/U55 including Transformer support.
- [ ] **Dynamic Level-of-Detail (LOD)**: Hot-swapping between functional and cycle-accurate models.

### 3. The Digital Twin: Multi-Physics & Time-Travel
**Objective**: Differentiate LabWired as a Cyber-Physical platform.
- [ ] **FMI 3.0 Native Support**: Integration with physical plant models (Battery, Thermal, Motors).
- [ ] **Distributed Time-Travel (D-TTD)**: Global snapshotting using Chandy-Lamport for multi-node fleets.
- [ ] **Instruction-Level Energy Profiling (ILEM)**: "Virtual Wattmeter" for Green Coding compliance.

### 4. Advanced Resilience: Security & Fault Injection
**Objective**: Enable autonomous red-teaming and security certification.
- [ ] **Virtual Fault Injection (VFI)**: Scriptable glitching (clock/voltage) for security bypass testing.
- [ ] **Side-Channel Emulation**: Power/EM trace generation (HW/HD models) for CPA analysis.
- [ ] **Rowhammer Simulation**: DRAM row-access modeling for memory vulnerability testing.

### 5. Scaling & Performance: Actor-Based Concurrency
**Objective**: Support fleet-scale simulation on modern heterogeneous hardware.
- [ ] **Lock-Free Actor Model**: Decoupling components into independent message-passing actors.
- [ ] **Linear Hardware Scaling**: Utilizing multi-core host machines without global locks.
