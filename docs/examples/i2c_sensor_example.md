# Example: STM32 I2C Sensor Interaction

This example demonstrates how to integrate and interact with a mock TMP102 temperature sensor in LabWired.

## 1. Registering the Peripheral

The `Tmp102` peripheral is implemented in `crates/core/src/peripherals/i2c_temp_sensor.rs`. It provides a simple register map:

| Offset | Register | Description |
|--------|----------|-------------|
| 0x00   | TEMP     | 12-bit Temperature Data (Read Only) |
| 0x04   | CONFIG   | Configuration Register (R/W) |
| 0x08   | T_LOW    | Lower Temperature Limit (R/W) |
| 0x0C   | T_HIGH   | Upper Temperature Limit (R/W) |

## 2. Configuring the System

To add the sensor to your simulation, you can define it in your `system.yaml` or directly in the chip descriptor.

### Via YAML Descriptor

```yaml
peripherals:
  - id: "temp_sensor"
    type: "i2c_temp_sensor"
    base_address: 0x4000_1000
    size: 0x100
```

*Note: In a more advanced simulation, the sensor would be connected behind an I2C controller. For this example, we map it directly to demonstrate register-level modeling.*

## 3. Interacting via Firmware

In your C/Rust firmware, you can read the temperature by accessing the memory-mapped address:

```c
#define TEMP_SENSOR_BASE 0x40001000
#define TEMP_REG_OFFSET  0x00

uint32_t read_temperature() {
    return *(volatile uint32_t*)(TEMP_SENSOR_BASE + TEMP_REG_OFFSET);
}
```

## 4. Verification in Simulator

When running the simulation, you can verify the sensor state using snapshots:

```bash
labwired run -f my_firmware.elf --snapshot snapshot.json
```

The resulting `snapshot.json` will contain:

```json
{
  "peripherals": {
    "temp_sensor": {
      "temp": 401,
      "config": 24736,
      "t_low": 1200,
      "t_high": 1280
    }
  }
}
```

## Summary
By using decoupled peripheral models like `Tmp102`, you can simulate complex digital sensor interactions without needing physical hardware, enabling earlier integration testing in your development cycle.
