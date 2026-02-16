# LabWired AI: Agentic Demos & Tests

This directory contains the verification pipelines and demos for LabWired's **Agentic Interfaces (AIPi)**.

---

## 🤖 Featured Demos

### [Autonomous Fix Demo](./autonomous_fix_demo.py)
**The "WOW" Demo.** showcasing how an agent can detect a bug in a hardware model and correct it autonomously using simulation feedback.
*   **Run**: `python3 autonomous_fix_demo.py`
*   **Proof**: Demonstrates the "Interface-First" iterative loop.

### [E2E Pipeline Test](./e2e_test.py)
A full-machine run that takes a datasheet, runs AI synthesis, generates a driver, and boots the simulation.
*   **Run**: `python3 e2e_test.py --device ADXL345 --datasheet <path>`

---

## 🧪 Technical Verification
These scripts are used by the development team to ensure the AI-to-Core bridge is reliable.

| Script | Purpose |
| :--- | :--- |
| `verify_device.py` | Generic stimulus-response checker for any YAML model. |
| `demo_dry_run.py` | Full release-gate dry run (Build -> CodeGen -> Simulation). |
| `true_e2e_test.py` | Deep verification of bitfield accuracy and register offsets. |

---

## 🚀 Getting Started
Ensure you have the Python dependencies installed:
```bash
cd .. && pip install -e .
```
(Note: Native engine bindings require `maturin develop` in `core/crates/python`).
