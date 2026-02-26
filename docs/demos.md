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
| **NUCLEO-H563ZI** | Our flagship demo: Absolute parity between simulation and the real physical board. | [`core/examples/nucleo-h563zi/`](./core/examples/nucleo-h563zi/README.md) |
| **Demo Blinky** | The classic "Hello World" of embedded systems, running on a modeled Cortex-M3. | [`core/examples/firmware-stm32f103-blinky/`](./core/examples/firmware-stm32f103-blinky/) |
| **RISC-V Virt** | Showcasing multi-architecture support via the `riscv-virt` machine. | [`core/examples/firmware-rv32i-hello/`](./core/examples/firmware-rv32i-hello/) |

---

## 🧪 3. CI & Automation Demos
Headless, deterministic execution for scaled regression testing.

| Demo | Description | Path |
| :--- | :--- | :--- |
| **UART Smoke Test** | Simple deterministic test runner (`labwired test`) example. | [`core/examples/ci/uart-ok.yaml`](./core/examples/ci/uart-ok.yaml) |
| **Workflow Templates** | Ready-to-use GitHub Actions and GitLab CI files. | [`core/examples/workflows/`](./core/examples/workflows/) |

---

## 🚀 How to Run
Most demos can be run via the CLI. Ensure you have the core built first:
```bash
cd core && cargo build --release
```

Then, follow the README in each specific demo directory.
