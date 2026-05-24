# LabWired Demos & Examples

Welcome to the LabWired Demo Portal. This directory helps you navigate the various ways to experience the platform, from high-fidelity hardware simulation to autonomous agentic workflows.

---

## 🤖 1. Agentic Interface Demos (The "WOW" Factor)
These demos showcase the **Interface-First** philosophy, where AI agents use LabWired as a high-fidelity sandbox to generate and verify hardware models.

| Demo | Description | Path |
| :--- | :--- | :--- |
| **Autonomous Fix** | Watch an agent detect a bug in a peripheral model and fix it via simulation feedback. | *Available in Commercial Tier* |
| **E2E Pipeline** | The full "Datasheet -> IR -> Codegen -> Simulation" path in one script. | *Available in Commercial Tier* |


---

## 🔌 2. Hardware Capability Demos
High-fidelity machine models showcasing peripheral accuracy and hardware parity.

| Demo | Description | Path |
| :--- | :--- | :--- |
| **NUCLEO-H563ZI** | Our flagship demo: Absolute parity between simulation and the real physical board. | [`examples/nucleo-h563zi/`](../examples/nucleo-h563zi/README.md) |
| **Demo Blinky** | The classic "Hello World" of embedded systems, running on a modeled Cortex-M3. | [`examples/demo-blinky/`](../examples/demo-blinky/) |
| **RISC-V Virt** | Showcasing multi-architecture support via the `riscv-virt` machine. | [`examples/riscv-virt/`](../examples/riscv-virt/) |
| **ESP32 E-Paper (Rust)** | ESP32-WROOM + SSD1680 tri-color e-paper, pure-Rust `esp-hal`. Sim partial, real hardware ✅. | [`examples/esp32-epaper-lab/`](../examples/esp32-epaper-lab/README.md) |
| **ESP32 E-Reader (Arduino)** | Same panel via Arduino-ESP32 + GxEPD2; sim paints end-to-end through FreeRTOS + ROM-thunk pipeline in v0.15.0. | [`examples/labwired-ereader-arduino/`](../examples/labwired-ereader-arduino/README.md) |
| **ESP32-S3 Blinky / Hello / I2C** | Minimal ESP32-S3 sketches for boot, USB-serial output, and TMP102 over I2C. | [`examples/esp32s3-blinky/`](../examples/esp32s3-blinky/), [`hello-world/`](../examples/esp32s3-hello-world/), [`i2c-tmp102/`](../examples/esp32s3-i2c-tmp102/) |
| **STM32 Tri-color E-Paper** | STM32F103 variant of the SSD1680 e-paper demo, byte-for-byte compatible. | [`examples/epaper-tricolor-lab/`](../examples/epaper-tricolor-lab/) |

---

## 🧪 3. CI & Automation Demos
Headless, deterministic execution for scaled regression testing.

| Demo | Description | Path |
| :--- | :--- | :--- |
| **UART Smoke Test** | Simple deterministic test runner (`labwired test`) example. | [`examples/ci/uart-ok.yaml`](../examples/ci/uart-ok.yaml) |
| **Workflow Templates** | Ready-to-use GitHub Actions and GitLab CI files. | [`examples/workflows/`](../examples/workflows/) |

---

## 🚀 How to Run
Most demos can be run via the CLI. Ensure you have the core built first:
```bash
cd core && cargo build --release
```

Then, follow the README in each specific demo directory.
