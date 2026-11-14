# ESP32-C5-DevKitC-1-N8R8 UART smoke

**Demand rank:** ESP32 is rank 1 in the measured MCU list (38%). The survey
does not rank ESP32-C5 separately. This adds a current C5 board profile, not a
claim that C5 has 38% usage. Current vendor status is in
[`vendor-status.md`](vendor-status.md).

## Copy-paste firmware example

The board's USB-to-UART bridge uses UART0. On v1.2, UART0 TX/RX appear on J3
pins TX/RX, mapped to GPIO11/GPIO12. This bare-metal example writes a short
message to UART0's FIFO:

```rust
#![no_std]
#![no_main]

use panic_halt as _;
use riscv_rt::entry;

const UART0_FIFO: *mut u32 = 0x6000_0000 as *mut u32;

#[entry]
fn main() -> ! {
    unsafe {
        for &byte in b"C5_N8R8_OK\n" {
            core::ptr::write_volatile(UART0_FIFO, u32::from(byte));
        }
    }
    loop { core::hint::spin_loop(); }
}
```

Run from the repository root:

```bash
examples/esp32c5-devkitc-1-n8r8/ci/build.sh
examples/esp32c5-devkitc-1-n8r8/ci/test.sh
```

## Verified scope

The chip descriptor maps the C5 HP RISC-V core, 8 MiB flash window, 384 KiB HP
SRAM, and the UART0 FIFO/status register head. The smoke checks that firmware
boots at its ELF entry and that UART0 transmits `C5_N8R8_OK` to the simulator
console.

The board pin map comes from Espressif's v1.2 guide. The simulator does not
model the C5 mask ROM, Espressif bootloader, GPIO/PCR, UART IO_MUX routing, the
physical USB-to-UART bridge, the 8 MiB PSRAM, the addressable RGB LED on GPIO27,
USB, Wi-Fi, Bluetooth, 802.15.4, or the LP core. The UART smoke verifies the
UART peripheral model and host capture. It does not prove the physical bridge
or pad route.
