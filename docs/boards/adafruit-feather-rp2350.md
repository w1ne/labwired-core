# Adafruit Feather RP2350 with HSTX

The Adafruit Feather RP2350 with HSTX (Product ID 6000, no-PSRAM option) uses
an RP2350 A4, 8 MiB QSPI flash, and 520 KiB SRAM. The RP2040/RP2350 family is
demand group #2; RP2040 was measured at 12% (MCU rank 5), while RP2350 was not
surveyed separately. The live product page listed the no-PSRAM board in stock
when checked on 2026-11-13; see the [availability receipt](../../examples/adafruit-feather-rp2350/evidence/vendor-status.md).

## Status at a glance

| Area | Status |
|---|---|
| Chip and board descriptors | RP2350 core profile with 8 MiB flash, 520 KiB SRAM, and GP0–GP29 pin labels |
| Board wiring | UART0 on GP0/GP1; active-high red D13 LED on GP7 |
| Firmware smoke | UART marker plus at least two SIO GPIO7 edges; committed local CLI evidence |
| Hosted twin verification | Not recorded for this board profile |
| Physical hardware comparison | Not recorded |

See the [board example](../../examples/adafruit-feather-rp2350/README.md),
[system descriptor](../../configs/systems/adafruit-feather-rp2350.yaml), and
[chip profile](../../configs/chips/rp2350-feather.yaml).

## Board pin map

| Feather pin | RP2350 pad | Use |
|---|---:|---|
| TX / RX | GP0 / GP1 | UART0 TX / RX |
| SDA / SCL | GP2 / GP3 | STEMMA QT, I2C1 SDA / SCL |
| D13 | GP7 | Onboard red LED, active high |
| HSTX connector | GP12–GP19 | HSTX-capable GPIO; HSTX model not included |
| NeoPixel | GP21 | Onboard addressable LED; model not included |
| SCK / MO / MI | GP22 / GP23 / GP20 | Main SPI0 pins |
| A0–A3 | GP26–GP29 | ADC inputs |

The pin map follows Adafruit's [live board pinout guide](https://learn.adafruit.com/adafruit-feather-rp2350/pinouts).

## Supported boundary

The local smoke executes a bare-metal Cortex-M33 firmware image on core 0. It
toggles GPIO7 through SIO and writes a marker through the PL011 UART0 model.
The committed smoke checks both observations against the exact Feather system
and chip profile.

HSTX/DVI, USB enumeration and USB CDC, the bootloader path, NeoPixel behavior,
charger behavior, and optional PSRAM are not represented by this smoke.
POWMAN/TICKS and TBMAN remain stubs in the shared RP2350 model. The test does
not establish physical-board behavior or validate the other peripherals in the
chip descriptor.
