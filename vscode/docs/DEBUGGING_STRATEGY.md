# Professional Debugging Strategy in LabWired

LabWired provides a first-class debugging experience designed specifically for embedded systems. Unlike traditional debuggers that rely solely on hardware probes, LabWired leverages its simulation engine to provide features that are difficult or impossible to achieve on physical hardware.

## Key Features

### 1. Reverse Debugging (Time-Travel)
LabWired allows you to "Step Back" through your code execution. By maintaining a high-fidelity instruction trace and periodic system checkpoints, you can reverse the simulation state to previous cycles to understand exactly where a bug originated.

- **Step Back**: Revert the last executed instruction, including register and memory side-effects.
- **Jump to Cycle**: Scrub the timeline to instantly teleport the simulation to any previous point in time.

### 2. High-Fidelity Timeline
The Timeline View provides a visual representation of your firmware's execution history.
- **Function Call Stack**: See which functions were running and how they nested over time.
- **Interrupt Markers**: Identify where hardware interrupts triggered and how long they took to process.
- **Instruction Density**: A heatmap showing performance bottlenecks and tight loops.

### 3. Peripheral & Register Inspector
Hardware-aware inspection using SVD (System View Description) data.
- **Nested Views**: Explore peripherals, registers, and bit-fields in a hierarchical tree.
- **Live Changed Highlighting**: Registers that changed since the last pause are highlighted in emerald green.
- **Hover Documentation**: Detailed descriptions for every register and field directly from the manufacturer's SVD.

### 4. RTOS Awareness
Deep integration with FreeRTOS and Zephyr.
- **Task List**: View the state (Running, Ready, Blocked), priority, and stack usage of all tasks.
- **Cross-Task Call Stacks**: Inspect the stack trace of blocked tasks without halting the core's real-time state.

## UI Design Philosophy

Our interface follows the "Professional simplicity" principle:
- **Cortex-Debug Inspired**: Clean, standard VS Code components. We don't reinvent the wheel; we make it roll smoother.
- **Functional Visibility**: Critical information (registers, stack, current instruction) should always be visible without deep nesting.
- **Modern Aesthetics**: Subtle micro-animations, glassmorphism for custom panels, and a premium "Ozone-class" feel that doesn't sacrifice performance.
- **Best Practices**: Adhere strictly to VS Code UX Guidelines for keyboard accessibility and consistent iconography.

## Verification & Testing Standards

Everything we build MUST be covered by automated tests. We employ a three-tier testing pyramid:

### 1. Unit Tests (Rust/TS)
- Low-level logic (e.g., SVD parsing, trace buffer management).
- Fast, deterministic, and isolated.
- **Required**: 100% logic coverage for all DAP/Core changes.

### 2. Integration Tests (Extension Host)
- Testing the interaction between VS Code and the DAP server.
- Verifies that commands, breakpoints, and state updates propagate correctly.
- Run using `@vscode/test-electron`.

### 3. End-to-End (E2E) Tests (Playwright)
- Visual regression tests for custom Webviews (Timeline, Logic Analyzer).
- Verifies the full user flow from opening a workspace to scrubbing the timeline.
- Ensures the UI remains "premium" across different OS versions and themes.
