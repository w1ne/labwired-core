# ESP8684-DevKitM-1-H4X

The `ESP8684-DevKitM-1-H4X` is an Espressif ESP32-C2-family board with a
single-core 32-bit RISC-V CPU and 4 MB flash. Its ESP32-C2 family is demand
rank 1 in the 2025 embedded survey; C2 is not reported separately. See the
[live lifecycle and availability receipt](../../examples/esp8684-devkitm-1-h4x/vendor-status.md).

## Current model scope

The system descriptor maps the board's 4 MB flash, 256 KiB data SRAM, and
UART0 FIFO/status register head. Its firmware smoke boots a bare ELF and
checks `ESP8684_H4X_UART0_OK` from UART0.

J3 pin 2 is GPIO20/U0TXD and J3 pin 3 is GPIO19/U0RXD, connected to the board's
USB-to-UART bridge. The simulator captures FIFO bytes internally; UART pin
routing and the physical bridge are not verified. The mask ROM, second-stage
bootloader, Wi-Fi, Bluetooth LE, GPIO/IO_MUX, ADC, I2C, SPI, timers, and RGB LED
are outside this profile.

## Copy-paste firmware

```rust
#![no_std]
#![no_main]

use panic_halt as _;
use riscv_rt::entry;

const UART0_FIFO: *mut u32 = 0x6000_0000 as *mut u32;

#[entry]
fn main() -> ! {
    unsafe {
        for &byte in b"ESP8684_H4X_UART0_OK\n" {
            core::ptr::write_volatile(UART0_FIFO, u32::from(byte));
        }
    }
    loop { core::hint::spin_loop(); }
}
```

Build/run commands, pin map, and evidence are in the
[board example](../../examples/esp8684-devkitm-1-h4x/README.md).
