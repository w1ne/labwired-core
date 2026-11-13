# ESP32-P4X-C5-Function-EV-Board UART0 smoke

**Demand rank:** ESP32 is rank 1 in the measured MCU list (38%). The survey
does not rank ESP32-P4 separately. This covers Espressif's newer P4 board; it
does not claim the family-wide 38% as P4-specific demand. Live vendor status
and the exact board variant are recorded in [`vendor-status.md`](vendor-status.md).

## Copy-paste firmware example

On the board's J1 header, pin 8 is GPIO37/U0TXD and pin 10 is GPIO38/U0RXD.
The bare-metal example writes a marker to UART0's FIFO at `0x500C_A000`:

```rust
#![no_std]
#![no_main]

use panic_halt as _;
use riscv_rt::entry;

const UART0_FIFO: *mut u32 = 0x500C_A000 as *mut u32;

#[entry]
fn main() -> ! {
    unsafe {
        for &byte in b"P4X_C5_FUNCTION_EV_OK\n" {
            core::ptr::write_volatile(UART0_FIFO, u32::from(byte));
        }
    }
    loop { core::hint::spin_loop(); }
}
```

Run from the repository root:

```bash
examples/esp32p4x-c5-function-ev-board/ci/build.sh
examples/esp32p4x-c5-function-ev-board/ci/test.sh
```

## Verified scope

The simulator loads this bare ELF at its entry point. The chip descriptor maps
the P4 HP RISC-V core's RV32IMAC subset, the 16 MB flash window, one 512 KiB
high-SRAM window, and the UART0 FIFO/status head. The smoke requires UART0 to
emit `P4X_C5_FUNCTION_EV_OK`.

This profile does not model reset ROM or the Espressif bootloader; the second
HP core; P4 floating-point or vector instructions; low HP SRAM; USB
Serial/JTAG; GPIO or IO_MUX pad routing; the ESP32-C5 module/radio; PSRAM; the
display, touch controller, camera, audio codec/amplifier, Ethernet PHY, USB
OTG, or MicroSD. UART0 output is captured inside the twin; the test does not
prove output on the physical header or USB port.
