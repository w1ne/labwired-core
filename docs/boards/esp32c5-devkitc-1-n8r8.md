# ESP32-C5-DevKitC-1-N8R8

**Demand rank:** ESP32 family is rank 1 in the JetBrains State of Embedded
2025 MCU survey (38%). The survey does not report a separate ESP32-C5 score.
This is a demand-aligned family addition; C5-specific usage is unmeasured.

**Vendor status, checked 2026-11-18:** Espressif's live product catalog lists
`ESP32-C5-DevKitC-1-N8R8` with distributor links and a $15 sample price
reference; DigiKey lists the exact board SKU as Active with 259 in stock. See the
[vendor-status receipt](../../examples/esp32c5-devkitc-1-n8r8/vendor-status.md).

## Pin map

| Board pin | SoC signal | LabWired profile |
|---|---|---|
| J3 TX | UART0 TX, GPIO11 | UART0 FIFO is modeled; pad routing is not |
| J3 RX | UART0 RX, GPIO12 | UART0 receive path and pad routing are not modeled |
| J3 pin 9 | GPIO5 | Direct GPIO output; exercised by the BME280 smoke |
| J1 pins 9/10 | GPIO8/GPIO9 | I2C0 SDA/SCL route; exercised by the BME280 smoke |
| J3 pin 7 | GPIO27, addressable RGB LED | Direct output and WS2812 frame decode exercised |

The profile models direct GPIO0..28 output and the C5 I2C0 command-list engine
with the GPIO8/9 matrix selectors used by the sensor smoke. Its RGB smoke
bit-bangs one red WS2812 frame on GPIO27 into the GPIO observer. The RMT
peripheral is not modelled. IO_MUX words are register-backed only; electrical
pad mux behavior and matrix functions beyond I2C0 are not claimed. GPIO27 is
also an ESP32-C5 strapping pin; the bare-ELF smoke does not model ROM sampling
at physical reset.

## Copy-paste example

Write a short message to the UART0 FIFO:

```rust
const UART0_FIFO: *mut u32 = 0x6000_0000 as *mut u32;

for &byte in b"C5_N8R8_OK\n" {
    unsafe { core::ptr::write_volatile(UART0_FIFO, u32::from(byte)); }
}
```

The full UART example and build commands are in
[`examples/esp32c5-devkitc-1-n8r8`](../../examples/esp32c5-devkitc-1-n8r8/README.md).
The sensor quickstart, with copy-paste Arduino code, is in
[`examples/esp32c5-devkitc-1-bme280-smoke`](../../examples/esp32c5-devkitc-1-bme280-smoke/README.md).

## Evidence and limits

The system descriptor maps the ESP32-C5 HP RISC-V core, the N8R8's 8 MiB flash
window, 384 KiB HP SRAM, UART0's FIFO/status head, direct GPIO output, and the
I2C0 command-list path needed by the BME280 example. The smoke boots a bare ELF
at its entry point, checks BME280 ID and raw-temperature reads, captures a
UART success marker, observes GPIO5 edges, and decodes one red frame from the
onboard GPIO27 WS2812 LED. It does not run the mask ROM or bootloader.

The UART0 register head uses Espressif's shared UART model at the C5's 240 MHz
core rate and interrupt source 47. The C5 GPIO adapter normalizes its different
register offsets and only enables the I2C0 matrix subset used by the smoke. The
simulator also omits UART clock gating, the UART tail, electrical IO_MUX
enforcement, the physical USB-to-UART bridge, PSRAM, RMT, USB, radios, and the
LP core.

Evidence logs: `examples/esp32c5-devkitc-1-n8r8/evidence/` and
`examples/esp32c5-devkitc-1-bme280-smoke/evidence/`.
