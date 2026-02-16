# LabWired Core Roadmap

This roadmap outlines the planned evolution of LabWired Core as we move towards a production-ready ecosystem for professional firmware simulation.

## 🟢 v0.1.0: Foundation (Current)
- **Multi-Architecture**: Initial ARM Cortex-M and RISC-V RV32I support.
- **Declarative Peripherals**: YAML-based chip and system definitions.
- **CI Test Runner**: Deterministic headless execution with JSON/JUnit reports.
- **Interactive Debugging**: DAP (VS Code) and GDB RSP integration.

## 🟡 v0.2.0: Ecosystem & Stability (Q1 2026)
- **Advanced SVD Ingestion**: Robust generation of register maps from standard CMSIS-SVD files.
- **Peripheral Expanded Set**: SPI, I2C Master, and DMA implementations for popular MCUs.
- **RTOS Awareness**: Initial task-list inspection for FreeRTOS and Zephyr.
- **Improved VS Code UX**: Dedicated register and memory windows in the Ozone-class extension.

## 🟠 v0.3.0: High-Fidelity Simulation (Q2 2026)
- **Multicore Support**: Independent execution loops for asymmetric/symmetric multicore SoCs.
- **Timing Accuracy**: Improved cycle models for pipeline stalls and bus contention.
- **Fault Injection API**: Programmatic induction of hardware faults for safety-critical testing.

## 🔴 v1.0.0: Enterprise Grade & Compliance
- **ISO 26262 Readiness**: Tool qualification kits and traceability reporting.
- **Cloud Fleet Execution**: Scalable, multi-tenant simulation orchestration.
- **AI-Accelerated Modeling**: Automated extraction of behavioral models from datasheets.

---

*Note: Dates and features are subject to change based on community feedback and project evolution.*
