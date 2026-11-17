# ESP32-C61-DevKitC-1-N8R2 UART and GPIO smoke

**Demand rank:** ESP32 family is rank 1 in the measured MCU list (38%). The
survey does not report a separate ESP32-C61 score. This round expands the
existing C61 board from UART-only to include its HP GPIO output-register path;
it does not claim that C61 itself has 38% usage. Current vendor availability is
recorded in [`vendor-status.md`](vendor-status.md).

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
`GPIO_OUT_REG` (`0x60091004`) and `GPIO_ENABLE_REG` (`0x60091034`).

## Verified scope

The chip profile maps the C61 HP RISC-V core, 8 MiB flash window, 320 KiB
on-chip HP SRAM, UART0 FIFO/status head, and the first 104 bytes of the HP GPIO
register window. The GPIO model covers the output latch and output enable used
by GPIO4. It does not cover the C61 GPIO matrix, IO_MUX, GPIO26..29, GPIO
interrupts, or electrical pad behavior. The onboard GPIO8 addressable RGB LED
protocol is not modeled.

Both smokes fast-load a bare ELF; they do not execute the mask ROM or Espressif
bootloader. The UART assertion does not prove the physical USB-to-UART bridge.
The 2 MiB PSRAM, USB, Wi-Fi, and Bluetooth LE remain outside this profile.

Evidence logs and structured results are in [`evidence/`](evidence/).
