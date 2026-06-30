# LabWired Core Roadmap

This roadmap outlines the planned evolution of LabWired Core as we move towards a production-ready ecosystem for professional firmware simulation.

## 🟢 v0.16.0: SoC Factory, Faithful ESP32 Boot & Module Refactor (Current)
- **SoC Factory architecture**: per-family peripheral factories + const peripheral table with a thin `from_config` match (generic, nRF52, ESP32 LX6/LX7); adding a chip is a table entry plus a factory hook.
- **Faithful ESP32 boot**: ESP32-S3 ROM auto-provisioning from the toolchain (loaded by default) and ESP32 (LX6) real-boot de-thunk; `Esp32s3BootMode` telemetry distinguishes Faithful vs Harness.
- **Silicon-validation & drift gate**: CI compares the model against silicon-derived expectations and fails on drift; auto-generated board status and a data-driven coverage matrix.
- **Core-accurate bus**: bit-band translation gated to Cortex-M3/M4 via an explicit `core` chip field (un-blocks STM32H563 / WBA52 GPIO); ESP32-S3 GDMA peripheral-coupled mode and corrected I2C0 interrupt source.
- **STM32H5 FDCAN** and a real-CAN UDS analyzer feeding the playground logic analyzer.
- **Module-split refactor**: bus, CLI, Xtensa, and WASM layers split into focused modules; fast PR CI gate with full suite post-merge.

## 🟢 v0.15.0: Dual-Core ESP32 + Arduino-ESP32 Bring-Up
- **Dual-Core ESP32 Simulation**: PRO_CPU / APP_CPU round-robin step loop, PRID register, cross-core IPI bridge for FreeRTOS yield.
- **Arduino-ESP32 / FreeRTOS Runtime**: ROM thunks for `xQueueCreateMutex*`, `xTaskGetCurrentTaskHandle`, `esp_clk_cpu_freq`, IPC-task no-ops, and SPIClass lazy `spi_t` init with USR_MOSI auto-enable — enough to boot a GxEPD2 sketch end-to-end through `setup()` → `drawPage()` → SSD1680 panel paint.
- **Runtime Snapshot Subsystem**: `Machine::{take,apply}_runtime_snapshot`, CLI `snapshot capture` subcommand, WASM `apply_runtime_snapshot` — cold-boot collapses from 30 s to ~0.5 s in the playground.
- **Xtensa Scheduler Fidelity**: `WSR.INTSET` raises pending IRQ bits so `portYIELD()` fires; `WSR.CCOMPARE0` acks-and-rearms so timer ticks land when CCOUNT has already overrun.

## 🟢 v0.14.0: Hardware-Validated Multi-Architecture Coverage
- **Hardware-Validated Parity**: STM32H563, STM32L476, and STM32F407 validation lanes with committed traces and oracle fixtures.
- **Multi-Architecture**: ARM Cortex-M, RISC-V RV32I/RV32A, and ESP32-S3 Xtensa LX7 execution paths.
- **Expanded Peripheral Coverage**: STM32 L4/F4 peripherals, ESP32-S3 GPIO/SYSTIMER/I2C support, and virtual I2C components.
- **CI Test Runner**: Deterministic headless execution with JSON/JUnit reports, trace fingerprints, and catalog validation metadata.
- **Interactive Debugging**: DAP (VS Code) and GDB RSP integration with conditional/data breakpoints and improved evaluation.

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
