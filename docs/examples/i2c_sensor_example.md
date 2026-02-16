# Example: I2C Sensor Simulation

This example demonstrates how to simulate an I2C peripheral (e.g., TMP102) and verify its behavior using standard Rust firmware.

## 1. System Configuration

The simulation environment requires defining the sensor in the system manifest and connecting it to the appropriate I2C controller.

### Configuration (`system.yaml`)
```yaml
chip: "../chips/stm32f103.yaml"
peripherals:
  - id: "i2c1"
    type: "i2c_master"
    base_address: 0x40005400

  - id: "tmp102"
    type: "i2c_temp_sensor"  # Instantiates the TMP102 model
    address: 0x48            # 7-bit I2C address
    bus: "i2c1"              # Connects to the I2C1 controller
```

## 2. Firmware Integration

The firmware uses standard HAL calls to interact with the simulated device. No simulation-specific code is required in the firmware itself.

### Rust Implementation (`stm32f1xx-hal`)
```rust
use stm32f1xx_hal::{i2c::{BlockingI2c, Mode}, pac};

fn main() -> ! {
    let dp = pac::Peripherals::take().unwrap();
    // ... Clock Configuration ...

    // Initialize I2C1 (Standard Mode, 100kHz)
    let mut i2c = BlockingI2c::i2c1(
        dp.I2C1,
        (scl, sda), // Pins PB6, PB7
        &mut afio.mapr,
        Mode::Standard { frequency: 100.kHz() },
        clocks,
        &mut rcc.apb1,
        1000, 10, 1000, 1000,
    );

    let sensor_addr = 0x48;
    let mut buffer = [0u8; 2];

    loop {
        // Read Temperature Register (0x00)
        i2c.write_read(sensor_addr, &[0x00], &mut buffer).unwrap();
        
        // Convert to Celsius (12-bit, 0.0625°C resolution)
        let raw_temp = u16::from_be_bytes(buffer) >> 4;
        let celsius = raw_temp as f32 * 0.0625;
    }
}
```

## 3. Automated Verification

LabWired supports scripted fault injection to verify error handling logic.

### Fault Injection Script (`tests/fault_test.yaml`)
Simulates a sensor failure or extreme environmental condition.

```yaml
steps:
  - run: 100ms
  - write_peripheral:
      id: "tmp102"
      reg: "TEMP"
      value: 0x7FF0 # Force sensor reading to 128°C
  - run: 10ms
  - assert_log: "CRITICAL: OVERTEMP DETECTED"
```

## 4. Execution

Run the simulation with the test script:

```bash
labwired test --script tests/fault_test.yaml
```

The simulator will execute the firmware, inject the fault at the specified time, and verify that the firmware correctly detects and logs the error condition.
