# ESP8684-DevKitM-1-H4X (ESP32-C2 family)

**Demand rank:** ESP32 is rank 1 in the measured MCU list (38%); the survey
does not report ESP32-C2 separately. The exact board SKU is
`ESP8684-DevKitM-1-H4X` with 4 MB flash. The older H4 board variant is marked
NRND by Espressif and is outside this profile.

## Pin map

| Board pin | Function | Current simulator scope |
| --- | --- | --- |
| J3 pin 2 (`TX`) | GPIO20 / UART0 TX; connected to the USB-to-UART bridge | UART0 FIFO output is captured inside the simulator; pad routing is not modeled |
| J3 pin 3 (`RX`) | GPIO19 / UART0 RX; connected to the USB-to-UART bridge | UART RX pad input is not modeled |
| J3 pin 6 (`8`) | GPIO8 / RGB LED (v1.0 addressable or v1.1 blue LED) | Not modeled |
| J1 pins 9–11 (`0`, `1`, `10`) | GPIO0, GPIO1, GPIO10 | Not modeled |

Pin names follow the [current v1.1 board guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c2/esp8684-devkitm-1/user_guide.html).

## Copy-paste firmware example

This bare-metal Rust image writes to the C2 UART0 FIFO at `0x6000_0000`:

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

Build and run from the repository root:

```bash
examples/esp8684-devkitm-1-h4x/ci/build.sh
examples/esp8684-devkitm-1-h4x/ci/test.sh
```

The simulator loads a bare ELF at its entry point, then verifies the UART0
FIFO/console marker. The profile does not execute mask ROM, the Espressif
bootloader, clock startup, or a physical UART bridge. Wi-Fi, Bluetooth LE,
GPIO/IO_MUX, ADC, I2C, SPI, timers, and the board RGB LED are not modeled here.

See the [live vendor status receipt](vendor-status.md) and the committed
[smoke evidence](evidence/).
