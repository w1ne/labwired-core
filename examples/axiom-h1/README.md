# AXIOM H1 — native ESP32-C3 firmware/physics bridge

This example contains the **real LabWired system manifest** used in the AXIOM H1
bidirectional firmware-in-the-loop tests. The companion Python runner, robot
MJCF, firmware source, acceptance scripts and measured traces are in the
LabWired application worktree:

`/data/axiom-h1/labwired/examples/axiom-h1/`

The Python extension in this branch adds two narrow functions to
`_native.NativeSession`:

- `inspect_json(peripheral=None)` uses the existing side-effect-free
  `Session.inspect()` contract. It returns modeled I²C and MCU observations,
  never inferred register values.
- `flash_path=` accepts the complete ESP32-C3 flash image and selects
  `BootMode::RomBoot` with the real provisioned ESP32-C3 mask-ROM images.
  The existing fast-boot ELF path remains supported; it now provisions
  ESP32-C3 ROM routines as it must for an Arduino application.

This repairs the native bridge's first failure at PC `0x40000018`, where a
normal ESP32-C3 application invoked a ROM routine but the Python runtime had
opened the chip with a zero-filled ROM map. There are no ROM instruction
stubs or overwritten application code.

Build the Python extension:

```sh
cargo build --release -p labwired-python
# Install liblabwired.so as _native.so in the Python environment
```

The AXIOM runner additionally observes GPIO8/9 SDA/SCL edges, decodes
ACKed address and register-write bytes, derives the 20 PCA9685 PWM targets,
reads GPIO3/4 output-enable state and injects actual MuJoCo IMU/rangefinder
values into the connected MCU at fixed 20 ms simulated-time steps.

These changes do **not** add MuJoCo to LabWired's firmware core. MuJoCo
is already bundled in KernelCAD Studio (browser version 3.9.0); the
AXIOM tests currently use a separate native MuJoCo 3.3.7 Python runtime.
No GPU, manufacturing, autonomous locomotion or physical-motor parity claim
is implied by a passing software simulation.
