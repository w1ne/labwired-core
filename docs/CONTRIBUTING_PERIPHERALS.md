# Contributing Peripheral Models to LabWired

Thank you for your interest in expanding the LabWired ecosystem! A simulation platform is only as useful as the hardware it can model. This document outlines the standards, philosophy, and review process for contributing new peripheral implementations to the `labwired-core` crate.

## 1. The LabWired Philosophy: Determinism and Fidelity

Unlike abstract or high-level emulators, LabWired is built to serve as a **Cyber-Physical Digital Twin**. Peripheral models must be written with the following core principles in mind:

*   **Determinism**: A simulation must yield the identical state (registers, memory, output traces) given the same inputs and instruction count, regardless of the host machine's speed.
*   **Performance via Actor Model**: Peripherals must not block the main execution loop. Heavy processing must be avoided, and state mutations should be highly optimized.
*   **Strict Memory Safety**: We use Rust to eliminate memory corruption bugs in the simulator. Unsafe blocks (`unsafe {}`) are strictly forbidden in peripheral code unless interacting with an explicitly approved lower-level abstraction (e.g., zero-copy IPC bridges for RTL co-simulation).
*   **State Serializability**: To support Distributed Time-Travel Debugging (via Chandy-Lamport snapshots) and Agentic "save-scumming" (fuzzing), all internal peripheral state must be perfectly serializable. Avoid raw pointers or non-deterministic data structures (like standard `HashMap` without a stable hasher).
*   **Dynamic Level-of-Detail (LOD)**: Code must be decoupled from the core execution engine to allow "hot-swapping" at runtime. A user might run your functional model for 10 seconds of boot time, then dynamically swap it for a cycle-accurate Verilator model for a specific driver interaction.

---

## 2. What Makes a "Good" Peripheral Model?

When reviewing pull requests for new peripherals, maintainers look for the following characteristics:

### 2.1 Functional vs. Cycle-Accurate
For the majority of community models, we target **Functional Accuracy**, augmented by instruction-level timing boundaries.
*   **Do**: Ensure that writing to an I2C transmit register sets the "TX Empty" flag after the expected number of bus ticks.
*   **Do**: Implement necessary side-effects (e.g., Read-to-Clear (RC) flags, Write-1-to-Clear (W1C)).
*   **Don't**: Attempt to model complex internal pipeline stalls or pipeline bubbles within a standard functional peripheral unless the specification explicitly defines them as observable state. For absolute cycle-precision, LabWired supports **RTL Co-Simulation** (Verilator integrations), which is governed by a separate process.

### 2.2 Event Emission & Physicality
Models should hook into the global event bus to facilitate headless CI assertions, VCD tracing, and external FMI 3.0 interactions.
*   Use standard traits to emit IRQs to the NVIC rather than hacking global states.
*   Trigger generic `Event::GpioEdge` or `Event::UartTx` rather than printing directly to `stdout`.
*   **Power States**: (Future-Proofing) If your peripheral has active/sleep modes that significantly affect power budgets, emit power state transition events to feed the Instruction-Level Energy Model (ILEM).

### 2.3 Idempotent Reads and Side-Effects
Hardware registers are notoriously tricky. Ensure you clearly separate:
*   `read()`: A pure read for the debugger (should have no side effects).
*   `read_side_effect()`: A read performed by the simulated CPU (which might clear status flags or advance FIFOs).

---

## 3. Contribution Workflow

1.  **Draft an Issue**: Before writing significant code, open a GitHub Issue proposing the peripheral. Define its scope (e.g., "STM32F4 Basic Timers TIM6/TIM7 only, excluding advanced PWM features").
2.  **Define the Schema**: Ensure the register layout is defined in a `peripheral.yaml` or imported cleanly from an SVD (System View Description) representation. LabWired heavily utilizes declarative macros to generate register boilerplate.
    *   *AI Assist*: If using the **FlexEmu** LLM pipeline, ensure your prompt maps the vendor C headers to LabWired's **9 Generic Primitives** (e.g., `Reg`, `RegField`, `Evt` for interrupts, `Upd` for hardware updates, `MemField` for DMA).
3.  **Implement**: Write the Rust model implementing the `Peripheral` trait in `crates/core/src/peripherals/`.
4.  **Write Tests**: This is the most critical step. (See section 4).

---

## 4. Testing and Acceptance Criteria

A peripheral **will not be merged** without accompanying regression tests demonstrating its fidelity against standard usage patterns.

### 4.1 Hardware Fidelity Tests
You must integrate your model into the `crates/core/tests/hardware_fidelity.rs` suite.
Your test must demonstrate:
*   **Register Read/Write capability** (including read-only and reserved bit masking).
*   **Side-effect triggering** (e.g., writing data starts an operation and raises an IRQ flag).
*   **Deterministic timing** (asserting that an operation completes after exactly `X` simulated CPU steps/cycles).

### 4.2 Performance Baselines
Reviewers will profile the peripheral. Implementations that cause significant regressions in the benchmark (Instructions Per Second) due to excessive allocations or nested locking will require optimization before merging.

### 4.3 AI-Generated Models (FlexEmu/Agents)
LabWired explicitly encourages the use of LLMs and Agentic workflows (like the MAESTRO framework) to bootstrap peripheral implementations from vendor C headers. 
*   **The Baseline Rule**: **AI-generated code is subject to the exact same rigorous standards as human-written code**. Reviewers will not accept models that "look right" but fail the `hardware_fidelity.rs` timing or side-effect assertions.
*   **Closed-Loop Verification**: If generating a model autonomously, ensure your agent includes a compilation and driver execution step to verify the generated primitive mappings before submitting a PR.

---

## 5. Versioning and Compatibility

LabWired adheres to Semantic Versioning as defined in the [Simulation Protocol Specification](./simulation_protocol.md).

*   **Binding to Protocol**: Your peripheral's behavior contributes to the overall determinism of the Simulation Protocol.
*   **Bug Fixes vs. Breaking Changes**:
    *   Fixing a bug that violates the vendor datasheet is considered a **patch**. While it changes simulation behavior, the previous behavior was incorrect by definition.
    *   Adding support for a completely new, major mode of the peripheral (e.g., adding DMA support to an existing SPI controller) should be flagged in the PR so test assertions can be updated globally.
*   **Deprecation**: If a peripheral implementation is structurally rewritten (e.g., moving from a naive polling model to an optimized event-driven model), the old model may be preserved using a `legacy` feature flag for one minor release cycle to allow users to migrate their CI test baselines.
