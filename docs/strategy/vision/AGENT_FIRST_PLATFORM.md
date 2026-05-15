[← Back to Hub](../../README.md)

# LabWired Platform Vision: The "Agent-First" Hardware Oracle

> **"Our main client is not even human anymore; it is an agent."**

## 1. The Paradigm Shift

Traditional firmware development tools (IDE, Debugger, Logic Analyzer) are built for **human eyes and hands**. They rely on GUIs, manual clicking, visual waveform inspection, and human intuition to spot anomalies.

**LabWired flips this model.**

We are building the first **Hardware Oracle**: a simulation platform designed primarily for **AI Agents** to interact with, observe, and debug hardware behavior. Humans are secondary observers.

## 2. Core Philosophy: "Remote Hands and Eyes"

For an agent to effectively debug firmware or write drivers, it needs more than just a text console. It needs structured, high-fidelity perception and control.

### 2.1. Determinism as a Prerequisite
Agents cannot reason about non-deterministic flakes. LabWired guarantees **bit-accurate reproducibility**.
- **Input**: A `system.yaml`, a firmware ELF, and a `script.yaml`.
- **Output**: Identical `result.json` and `trace.vcd` every single time.
- **Benefit**: This allows agents to confidently perform "Root Cause Analysis" without worrying about environmental noise.

### 2.2. Structured Observability (No Screen Scraping)
Humans read logs; Agents read **JSON**.
- **State**: Instead of a register view window, we provide a JSON snapshot of the entire memory map.
- **Trace**: Instead of a waveform GUI, we provide VCD and JSON event streams.
- **Errors**: Instead of a localized GUI popup, we provide structured error codes and context in `result.json`.

### 2.3. The Agent Economy (Metering)
Agents consume compute resources. To enable a future "Simulation-as-a-Service" economy for agents, LabWired includes native **Metering**.
- **Metric**: `Simulation Minutes` (or `Total Instructions Retired`).
- **Goal**: Allow fleet managers to budget agent entitlements (e.g., "This fix is allowed 10M cycles of verification").

## 3. Architecture Pillars

### I. The Headless Oracle (Core)
- **Rust-based Engine**: High performance, no GUI dependencies.
- **Declarative Hardware**: Chips and boards defined in simple YAML, allowing agents to "hallucinate" new hardware configurations easily.

### II. The API for Agents (AIPi)
- **Validation**: `labwired asset validate` allows agents to check their own work.
- **Audit**: `unsupported_instruction_audit` allows agents to self-discover simulator limitations.
- **Feedback**: Closed-loop stimulus-response via Python bindings (`labwired_ai`).

### III. The Human Window (VS Code)
Humans only step in to verify the agent's work. The VS Code extension is a **viewer** for the artifacts the agent produced, not the primary driver.

## 4. Strategic Roadmap

1.  **Phase 1: Foundation**: Deterministic engine, JSON outputs, initial Python bindings (Done).
2.  **Phase 2: Agentic Loop**: Autonomous fix demos, datasheet-to-simulation pipeline (In Progress).
3.  **Phase 3: Hosted CI**: API-key-gated paid CI tier — Cloudflare Worker + Stripe + GitHub Action ([packages/api](../../../packages/api/)). Live in beta. (The earlier "Foundry" hosted-API framing has been retired in favour of this simpler CI surface.)
