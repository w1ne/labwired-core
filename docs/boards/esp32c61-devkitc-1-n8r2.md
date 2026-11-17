# ESP32-C61-DevKitC-1-N8R2

**Demand rank:** ESP32 family is rank 1 in the JetBrains State of Embedded
2025 MCU survey (38%). The survey does not report a C61-specific percentage.
The current C61 variant is listed in Espressif's live board guide and catalog;
see the [availability receipt](../../examples/esp32c61-devkitc-1-n8r2/vendor-status.md).

| Artifact | Path |
|---|---|
| Chip descriptor | [`configs/chips/esp32c61.yaml`](../../configs/chips/esp32c61.yaml) |
| Board system | [`configs/systems/esp32c61-devkitc-1-n8r2.yaml`](../../configs/systems/esp32c61-devkitc-1-n8r2.yaml) |
| Firmware and smokes | [`examples/esp32c61-devkitc-1-n8r2`](../../examples/esp32c61-devkitc-1-n8r2/README.md) |

## Pin map and profile

| Board pin | SoC signal | LabWired profile |
|---|---|---|
| J3 TX | UART0 TX, GPIO11 | UART0 FIFO is modeled; pad routing is not |
| J3 RX | UART0 RX, GPIO10 | UART0 receive and pad routing are not modeled |
| J1 pin 3 | GPIO4 | Output latch and enable register are modeled; IO_MUX and electrical behavior are not |
| J1 pin 5 | GPIO6 | I2C0 SDA for the BME280 smoke; GPIO matrix route is modeled |
| J3 pin 6 | GPIO22 | I2C0 SCL for the BME280 smoke; GPIO matrix route is modeled |
| J1 pin 9 | GPIO8, addressable RGB LED | LED protocol is not modeled |

The GPIO output smoke checks bit 4 in `GPIO_OUT_REG` and `GPIO_ENABLE_REG` after
firmware sets GPIO4. The BME280 smoke configures GPIO6/GPIO22 for I2C0 and reads
the device ID (`0x60`) plus a deterministic raw-temperature test vector. The
GPIO model covers those C61 matrix selectors, but not other peripheral matrix
routes, GPIO26..29, GPIO interrupts, or electrical pad behavior. IO_MUX values
are stored but not enforced. The board's addressable RGB LED on GPIO8 is also
outside scope.

The board profile fast-loads a bare ELF directly; it does not execute the
mask-ROM boot path or Espressif bootloader. The UART test captures FIFO output
in the host console but does not model the physical USB-to-UART bridge.

## GPIO copy-paste example

```rust
const GPIO_OUT_W1TS: *mut u32 = 0x6009_1008 as *mut u32;
const GPIO_ENABLE_W1TS: *mut u32 = 0x6009_1038 as *mut u32;
const GPIO4: u32 = 1 << 4;

unsafe {
    core::ptr::write_volatile(GPIO_ENABLE_W1TS, GPIO4);
    core::ptr::write_volatile(GPIO_OUT_W1TS, GPIO4);
}
```

The full firmware and build/test commands are in
[`examples/esp32c61-devkitc-1-n8r2`](../../examples/esp32c61-devkitc-1-n8r2/README.md).
The GPIO smoke expects bits 4 in `GPIO_OUT_REG` and `GPIO_ENABLE_REG`; the UART
smoke expects `C61_N8R2_OK`. Logs and structured results are under the example's
`evidence/` directory.

The BME280 wiring, copy-paste ESP-IDF example, fresh smoke, scope, and live
availability receipt are documented in
[`examples/esp32c61-devkitc-1-n8r2-bme280`](../../examples/esp32c61-devkitc-1-n8r2-bme280/README.md).
