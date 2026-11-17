# ESP32-C61-DevKitC-1-N8R2 UART, GPIO, and BME280 smokes

**Demand rank:** ESP32 family is rank 1 in the measured MCU list (38%). The
survey does not report a separate ESP32-C61 score. This round expands the
existing C61 board with a BME280 I2C path on GPIO6/GPIO22; earlier UART0 and
GPIO4 output-register smokes remain. The survey does not report a separate
C61 score. Live board and sensor availability is recorded in the [BME280
receipt](../esp32c61-devkitc-1-n8r2-bme280/vendor-status.md).

## Copy-paste firmware example

On the v2.0 board, GPIO4 is J1 pin 3. The sample enables its output latch and
sets the output bit, then writes a marker to UART0. LabWired checks the GPIO
register state and captures the UART FIFO; it does not simulate IO_MUX routing
or electrical behavior at the physical pin.

```rust
#![no_std]
#![no_main]

use panic_halt as _;
use riscv_rt::entry;

const GPIO_OUT_W1TS: *mut u32 = 0x6009_1008 as *mut u32;
const GPIO_ENABLE_W1TS: *mut u32 = 0x6009_1038 as *mut u32;
const UART0_FIFO: *mut u32 = 0x6000_0000 as *mut u32;
const GPIO4: u32 = 1 << 4;

#[entry]
fn main() -> ! {
    unsafe {
        core::ptr::write_volatile(GPIO_ENABLE_W1TS, GPIO4);
        core::ptr::write_volatile(GPIO_OUT_W1TS, GPIO4);
        for &byte in b"C61_N8R2_OK\n" {
            core::ptr::write_volatile(UART0_FIFO, u32::from(byte));
        }
    }
    loop { core::hint::spin_loop(); }
}
```

Run from the repository root:

```bash
rustup target add riscv32imc-unknown-none-elf
examples/esp32c61-devkitc-1-n8r2/ci/build.sh
examples/esp32c61-devkitc-1-n8r2/ci/test.sh
```

The UART test expects `C61_N8R2_OK`. The GPIO test checks bit 4 in both
`GPIO_OUT_REG` (`0x60091004`) and `GPIO_ENABLE_REG` (`0x60091034`). The chip
descriptor connects UART0 to the modeled C61 PCR `UART0_CONF.CLK_EN` gate.

## BME280 I2C example

Connect a BME280 breakout at address `0x77` with SDA on GPIO6 (J1 pin 5) and
SCL on GPIO22 (J3 pin 6). The [sensor example](../esp32c61-devkitc-1-n8r2-bme280/README.md)
includes a copy-paste ESP-IDF `app_main`, smoke command, live availability
receipt, and committed output. Its firmware reads the sensor ID and a raw
temperature vector over the C61 GPIO matrix route.

```bash
examples/esp32c61-devkitc-1-n8r2-bme280/ci/test.sh
```

## Verified scope

The chip profile maps the C61 HP RISC-V core, 8 MiB flash window, 320 KiB
on-chip HP SRAM, UART0 FIFO/status head, GPIO4 output state, and the C61 GPIO
matrix selectors needed for I2C0 on GPIO6/GPIO22. It also models the UART0 and
I2C0 PCR clock gates. The BME280 smoke verifies register reads and route
selection. IO_MUX is storage only; electrical pad behavior, GPIO26..29, other
matrix functions, GPIO interrupts, and the onboard GPIO8 addressable RGB LED
protocol are outside this profile.

All three smokes fast-load a bare ELF; they do not execute the mask ROM or
Espressif bootloader. The UART assertion does not prove the physical USB-to-UART bridge.
The 2 MiB PSRAM, USB, Wi-Fi, and Bluetooth LE remain outside this profile.

UART/GPIO evidence logs are in [`evidence/`](evidence/); BME280 evidence and
the live availability receipt are in the linked sensor example.
