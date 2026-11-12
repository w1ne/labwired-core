# ESP32-C5-DevKitC-1-N8R8

**Demand rank:** ESP32 family is rank 1 in the JetBrains State of Embedded
2025 MCU survey (38%). The survey does not report a separate ESP32-C5 score.
This is a demand-aligned family addition; C5-specific usage is unmeasured.

**Vendor status, checked 2026-11-12:** Espressif's current v1.2 guide lists
`ESP32-C5-DevKitC-1-N8R8` as an available retail-sample variant. See the
[vendor-status receipt](../../examples/esp32c5-devkitc-1-n8r8/vendor-status.md).

## Pin map

| Board pin | SoC signal | LabWired profile |
|---|---|---|
| J3 TX | UART0 TX, GPIO11 | UART0 FIFO is modeled; pad routing is not |
| J3 RX | UART0 RX, GPIO12 | UART0 receive path and pad routing are not modeled |
| J3 pin 27 | GPIO27, addressable RGB LED | Not modeled |

Espressif also breaks out the available GPIO pins on J1/J3. The simulator does
not currently model the C5 GPIO block, so this profile does not expose GPIO as
a working peripheral.

## Copy-paste example

Write a short message to the UART0 FIFO:

```rust
const UART0_FIFO: *mut u32 = 0x6000_0000 as *mut u32;

for &byte in b"C5_N8R8_OK\n" {
    unsafe { core::ptr::write_volatile(UART0_FIFO, u32::from(byte)); }
}
```

The full bare-metal example and build commands are in
[`examples/esp32c5-devkitc-1-n8r8`](../../examples/esp32c5-devkitc-1-n8r8/README.md).

## Evidence and limits

The system descriptor maps the ESP32-C5 HP RISC-V core, the N8R8's 8 MiB flash
window, 384 KiB HP SRAM, and UART0's FIFO/status head. The smoke boots the ELF
at its entry point and checks UART output. It does not run the mask ROM or
bootloader.

The UART0 register head uses Espressif's shared UART model at the C5's 240 MHz
core rate. The model uses C5 interrupt source 47. The C5 GPIO offsets differ
from C3/C6, so the existing GPIO model is not reused. The simulator also omits
UART clock gating, the UART tail, pad electrical behavior, the physical
USB-to-UART bridge, PSRAM, RGB LED protocol, USB, radios, and the LP core.

Evidence log: `examples/esp32c5-devkitc-1-n8r8/evidence/`.
