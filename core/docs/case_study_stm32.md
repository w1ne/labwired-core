# Case Study: Debugging STM32 Without Hardware

## The Problem

Embedded development typically requires physical hardware, even for basic testing. This leads to several challenges:
- **Bottlenecks**: Teams waiting for prototype hardware to arrive.
- **Complexity**: Debugging I2C/SPI timing issues requires logic analyzers or oscilloscopes.
- **CI/CD Gaps**: Hard to run automated tests for firmware on real hardware in every PR.

## The LabWired Solution

LabWired allows developers to simulate an entire STM32-based system, including external peripherals like sensors, directly in their local environment and VS Code.

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

- **Zero Setup Costs**: No hardware, cables, or probes required.
- **Instant Reproducibility**: High-fidelity simulation ensures a bug in simulation is a bug in hardware.
- **Modern Workflow**: Bring the power of modern software development (CI/CD, fast iteration) to the embedded world.

---

## Technical Highlights

- **Cortex-M3 Core**: High-fidelity instruction execution.
- **Declarative Peripherals**: Easily add new sensors by writing YAML descriptors.
- **DAP Integration**: Seamless connection to VS Code.

[Back to LabWired Core](../README.md)
