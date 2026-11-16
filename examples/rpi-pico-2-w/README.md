# Raspberry Pi Pico 2 W

RP2350 + Infineon CYW43439 on the PIO gSPI bus. System manifest:
`configs/systems/rpi-pico-2-w.yaml`. Arduino matrix row: board `rpipico2w`
(`validation/arduino-matrix/boards/rpipico2w.yaml`).

The onboard LED is the CYW43439's `WL_GPIO0`, not an RP2350 pad. On this
board `digitalWrite(LED_BUILTIN, ...)` is a full chip bring-up: the stock
arduino-pico core downloads the 224 KiB WLAN firmware over gSPI (PIO + DMA),
releases the chip's Arm core, then sends a `gpioout` iovar. Watch it as
`cyw43:0`. The CYW43439 model is the one the Pico W uses.

## Evidence

`evidence/arduino-matrix/` — stock arduino-pico, pinned
`maxgerhardt/platform-raspberrypi@5d4561a0`, generated 2026-11-16:

| Sketch | Result | Oracle |
| --- | --- | --- |
| L0 serial boot | pass | UART marker |
| L1 serial loop | pass | UART marker |
| L2 blink | pass | UART marker + 2 edges on `cyw43:0` (first edge at cycle 61.23M) |
| L3 I2C (INA219) | pass | UART marker |
| L4 SPI (MAX31855) | pass | UART marker |
| L5 ADC | pass | UART marker |
| L6 PWM | pass | 147 edges on GP15 (PWM slice 7 B) |
| L7 timer | pass | UART marker |
| L8 CAN | skipped | RP2350 has no CAN controller |

## What is not modelled

The CYW43439 model answers the host interface of a running WLAN firmware.
It does not execute that firmware: no radio, scan, join, data traffic,
async events or Bluetooth. Other `GET` ioctls answer zeros. The RP2350 DMA
security and MPU registers (`SECCFG_*`, `MPU_*`) are not modelled.
