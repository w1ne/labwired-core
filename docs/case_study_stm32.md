# Case Study: Debugging STM32 Without Hardware

## The Problem

Embedded development typically requires physical hardware, even for basic testing. This leads to several challenges:
- **Bottlenecks**: Teams waiting for prototype hardware to arrive.
- **Complexity**: Debugging I2C/SPI timing issues requires logic analyzers or oscilloscopes.
- **CI/CD Gaps**: Hard to run automated tests for firmware on real hardware in every PR.

## The LabWired Solution

LabWired runs STM32 firmware against supported MCU peripheral and external-device models, including I²C sensors. The example is a model-based firmware test, not a complete analog or physical replica of the board. See [what a green pass means](fidelity.md) and the [target support levels](target_support_rubric.md).

### Scenario: I2C Temperature Sensor Integration

In this demo, we integrated a **TMP102** temperature sensor with an **STM32F103** microcontroller.

#### 1. Hardware-Free Simulation
Instead of wiring physical pins, we defined the sensor and its connection in a declarative YAML file:

```yaml
# system.yaml
external_devices:
  - id: "temp_sensor"
    type: "tmp102"
    connection: "i2c1"
    config:
      i2c_address: 0x48
```

#### 2. Realistic Firmware
The firmware uses standard STM32 peripheral registers (RCC, GPIO, I2C) to communicate with the virtual sensor. The emulator handles these register accesses and routes them to the virtual device model.

#### 3. One-Click Debugging
Using the LabWired VS Code extension, the developer can:
- **Set Breakpoints**: Pause execution exactly where the sensor is being read.
- **Inspect Registers**: View the status of the I2C control registers and the data register.
- **Step Through Logic**: Verify that the temperature conversion logic handles negative values or overflow correctly.

## Impact

- **No bench required for the model check**: Run supported firmware and device-model tests without a connected board. Physical hardware is still required for acceptance tests.
- **Repeatable evidence**: Reuse the same firmware, system manifest and inputs to reproduce an observed model failure. A simulated failure does not automatically prove a silicon defect; a simulated pass does not prove the physical board works.
- **CI integration**: Assert supported MCU and I²C device behavior on every change, then use physical tests to cover unmodeled electrical and timing effects.

Record the firmware revision, model version, inputs, expected results and simulator artifacts. Label the result **model-verified** only when its explicit assertions pass; this is not a whole-board or hardware-acceptance verdict.

---

## Technical Highlights

- **Cortex-M3 Core**: High-fidelity instruction execution.
- **Declarative Peripherals**: Easily add new sensors by writing YAML descriptors.
- **DAP Integration**: Seamless connection to VS Code.

[Back to LabWired Core](../README.md)
