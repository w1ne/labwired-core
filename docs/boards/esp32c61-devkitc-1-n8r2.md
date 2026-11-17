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
| J1 pin 9 | GPIO8, addressable RGB LED | LED protocol is not modeled |

The GPIO output smoke checks bit 4 in `GPIO_OUT_REG` and `GPIO_ENABLE_REG` after
firmware sets GPIO4. The GPIO model covers the C61 HP output-register slice
through GPIO4. The C61 GPIO matrix, IO_MUX, GPIO26..29, GPIO interrupts, and
electrical pad behavior remain outside the profile. The board's addressable
RGB LED on GPIO8 is also outside scope.

The board profile fast-loads a bare ELF directly; it does not execute the
mask-ROM boot path or Espressif bootloader. The UART test captures FIFO output
in the host console but does not model the physical USB-to-UART bridge.

## Copy-paste example

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
