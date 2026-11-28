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
SRAM, UART0 FIFO/status register head, direct GPIO output, and the C5 I2C0
subset used by the sensor example. This UART-only smoke checks that firmware
boots at its ELF entry and that UART0 transmits `C5_N8R8_OK` to the simulator
console. The stronger sensor smoke reads the BME280 ID and raw temperature and
observes GPIO5 edges; see
[`examples/esp32c5-devkitc-1-bme280-smoke`](../esp32c5-devkitc-1-bme280-smoke/README.md).

The board pin map comes from Espressif's v1.2 guide. The simulator does not
model the C5 mask ROM, Espressif bootloader, PCR clock gating, UART IO_MUX
routing, the physical USB-to-UART bridge, the 8 MiB PSRAM, the addressable RGB
LED on GPIO27, USB, Wi-Fi, Bluetooth, 802.15.4, or the LP core. The UART smoke
checks the UART peripheral model and host capture. It does not prove the
physical bridge or pad route.

## Analog inputs

`chips/esp32c5.yaml` publishes the ADC1 pads in `analog_pins:`. GPIO1..GPIO5
are ADC1_CH0..CH4 (ESP-IDF `components/soc/esp32c5/include/soc/adc_channel.h`).
The pad number is not the channel number on this chip. The `apb_saradc`
instance is the ESP32-C3 one-shot SAR engine at `0x6000E000`, because the C5
register head matches it.

[`analog-ntc-diode-system.yaml`](analog-ntc-diode-system.yaml) is the generic
Beta NTC divider on GPIO2 and Shockley diode load line on GPIO4. Both C5
profiles run it unchanged:

```bash
cargo test -p labwired-core --test analog -- esp32c5
```

The test solves both netlists at -20, 25 and 85 C, converts ADC1 channels 1
and 3 through the `adc_oneshot_read` register sequence, and checks that the
router refuses GPIO0, GPIO7 and GPIO8.

Gaps: GPIO6 (ADC1_CH5) is not published, because the shared model reports
the C3's five inputs. Raising that count is a change to a file that the
silicon-validated C3 and C6 boards watch, so it waits for a drift ack. The
converter has no PCR clock gate, the register reset seeds are the C3 capture,
and attenuation is not modelled.
