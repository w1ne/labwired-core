# ESP32-C61-DevKitC-1-N8R2

**Demand rank:** ESP32 family is rank 1 in the JetBrains State of Embedded
2025 MCU survey (38%). The survey does not report a C61-specific percentage.
The current C61 variant is listed in Espressif's live board guide and
distributor catalog; see the [availability receipt](../../examples/esp32c61-devkitc-1-n8r2/vendor-status.md).

| Artifact | Path |
|---|---|
| Chip descriptor | [`configs/chips/esp32c61.yaml`](../../configs/chips/esp32c61.yaml) |
| Board system | [`configs/systems/esp32c61-devkitc-1-n8r2.yaml`](../../configs/systems/esp32c61-devkitc-1-n8r2.yaml) |
| Firmware and smoke | [`examples/esp32c61-devkitc-1-n8r2`](../../examples/esp32c61-devkitc-1-n8r2/README.md) |

## Pin map and profile

| Board pin | SoC signal | LabWired profile |
|---|---|---|
| J3 TX | UART0 TX, GPIO11 | UART0 FIFO is modeled; pad routing is not |
| J3 RX | UART0 RX, GPIO10 | UART0 receive and pad routing are not modeled |
| J1 pin 9 | GPIO8, addressable RGB LED | Not modeled |

The smoke profile includes only the C61 UART0 FIFO/status head and captures
transmitted bytes in the host console. It fast-loads an ELF directly, without
executing the mask ROM or bootloader. GPIO, PCR, IO_MUX, the physical bridge,
PSRAM, USB, Wi-Fi and Bluetooth LE are outside the profile.

## Copy-paste example

```rust
const UART0_FIFO: *mut u32 = 0x6000_0000 as *mut u32;

for &byte in b"C61_N8R2_OK\n" {
    unsafe { core::ptr::write_volatile(UART0_FIFO, u32::from(byte)); }
}
```

The full firmware and build/test commands are in
[`examples/esp32c61-devkitc-1-n8r2`](../../examples/esp32c61-devkitc-1-n8r2/README.md).
The UART smoke expects `C61_N8R2_OK`; its committed logs and structured result
are under that example's `evidence/` directory.
