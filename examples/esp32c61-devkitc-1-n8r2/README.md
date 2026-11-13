# ESP32-C61-DevKitC-1-N8R2 UART smoke

**Demand rank:** ESP32 is rank 1 in the measured MCU list (38%). The survey
does not report a separate ESP32-C61 score. This adds a current C61 board to
the family; it does not claim that C61 itself has 38% usage. Current vendor
availability is recorded in [`vendor-status.md`](vendor-status.md).

## Copy-paste firmware example

The board's USB-to-UART bridge is connected to UART0. Espressif's v2.0 board
guide maps J3 TX/RX to GPIO11/GPIO10. The simulator captures the UART0
peripheral internally, so the example writes to its FIFO:

```rust
#![no_std]
#![no_main]

use panic_halt as _;
use riscv_rt::entry;

const UART0_FIFO: *mut u32 = 0x6000_0000 as *mut u32;

#[entry]
fn main() -> ! {
    unsafe {
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

The committed UART assertion expects `C61_N8R2_OK`.

## Verified scope

The chip profile maps the C61 HP RISC-V core, 8 MiB flash window, 320 KiB
on-chip HP SRAM, and the UART0 FIFO/status register head. The smoke loads a bare
ELF at its entry and checks UART0 output in the simulator host console. It does
not execute the mask ROM or Espressif bootloader.

The physical UART pins are documented, but GPIO, IO_MUX pad routing, the
USB-to-UART bridge, the 2 MiB PSRAM, the GPIO8 addressable RGB LED, USB, Wi-Fi,
Bluetooth LE, and other peripheral blocks are not modeled by this board
profile. The UART assertion proves the UART FIFO/console model only; it does
not prove the physical bridge or pad route.

Evidence logs: [`evidence/`](evidence/).
