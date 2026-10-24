# Calliope mini 1.x (Nordic nRF51822) — system manifest only

The **Calliope mini 1.x** uses the same MCU as the BBC micro:bit V1: Nordic
**nRF51822** — Cortex-M0 at 16 MHz with an S110 SoftDevice below the
application. LabWired reuses the micro:bit V1 chip descriptor and models the
board as a second system manifest on it.

| Aspect | Status |
|--------|--------|
| Chip descriptor | [`configs/chips/nrf51822.yaml`](../../configs/chips/nrf51822.yaml) (shared with [micro:bit V1](microbit-v1.md)) |
| System | [`configs/systems/calliope-mini-v1.yaml`](../../configs/systems/calliope-mini-v1.yaml) |
| Smoke script, committed ELF, survival gate | none |
| Tier | **none**: a manifest, not a qualified target |

## What the manifest states

Buttons A (P0.17) and B (P0.16), the host UART (P0.24 / P0.25) and the 5x5
LED matrix on the micro:bit V1's row and column lines with the same map. The
motor driver is not a modelled device: its three inputs (nSLEEP P0.28, IN1
P0.29, IN2 P0.30) are ordinary GPIO lines a host can watch.

The pin facts come from the board's open runtime
([calliope-mini/microbit-dal](https://github.com/calliope-mini/microbit-dal),
MIT) and its mbed target pin names (Apache-2.0). None was checked against
hardware or a schematic.

## Not modelled / not claimed

The motor driver and any motor, the speaker, the RGB LED, the microphone, the
accelerometer/magnetometer/gyroscope, the ADC and the Grove connectors. No
labwired-core lane runs firmware on this manifest, so there is no boot, radio,
timing or fidelity claim for the board; official MakeCode images additionally
need the SoftDevice emulation, as on the micro:bit V1.
