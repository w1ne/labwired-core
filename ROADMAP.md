# LabWired Core Roadmap

This roadmap outlines the planned evolution of LabWired Core as we move towards a production-ready ecosystem for professional firmware simulation.

## 🟢 v0.1.0: Foundation (Current)
- **Multi-Architecture**: Initial ARM Cortex-M and RISC-V RV32I support.
- **Declarative Peripherals**: YAML-based chip and system definitions (The "Zero to One" Enabler).
- **CI Test Runner**: Deterministic headless execution with JSON/JUnit reports.
- **Interactive Debugging**: DAP (VS Code) and GDB RSP integration.

## 🟡 v0.2.0: The Agentic Interface (Q1 2026)
- **Python Bindings**: Direct memory/register access for Python-based AI Agents (PyTorch/Gym).
- **Snapshot Fuzzing**: API to fork simulation state for parallel path exploration.
- **Advanced SVD Ingestion**: Robust generation of register maps to feed the declarative engine.

## 🟠 v0.3.0: Closing the Library Gap (Q2 2026)
- **AI-Datasheet-Importer**: "Infinite Library" engine – automated generation of peripheral YAML from PDF datasheets.
- **Timing Accuracy**: Improved cycle models for pipeline stalls and bus contention.
- **Multicore Support**: Independent execution loops for asymmetric/symmetric multicore SoCs.

## 🔴 v1.0.0: Enterprise Grade & Compliance
- **ISO 26262 Readiness**: Tool qualification kits and traceability reporting.
- **Swarm Simulation**: Multi-process orchestration to simulate networks/fleets.
- **Cloud Fleet Execution**: Scalable, multi-tenant simulation orchestration.

---

*Note: Dates and features are subject to change based on community feedback and project evolution.*
