# Required Documents — Arduino Nano 33 BLE

Authoritative sources used by this example (retrieved 2026-10-23):

| Doc | Title | Used for |
| :--- | :--- | :--- |
| Arduino ABX00030 / ABX00069 schematic | Nano 33 BLE (Rev2) schematic | RGB LED on P0.24/P0.16/P0.06 (cathodes to the pins → active low), built-in D13 LED on P0.13, `Serial1` on P1.03/P1.10, LSM9DS1 on TWI0 (SDA P0.14 / SCL P0.15) |
| ArduinoCore-mbed `variants/ARDUINO_NANO33BLE` | the board pin map | `LEDR`/`LEDG`/`LEDB` = P0.24/P0.16/P0.06; `D0/TX` = P1.03, `D1/RX` = P1.10 |
| Arduino Help Center — "Control the RGB LED on Nano 33 BLE boards" | board LED behaviour | "The RGB pins are active-low" |
| nRF52840 Product Specification (v1.11) | the SoC | UARTE0 @ 0x4000_2000 (EasyDMA TXD.PTR/MAXCNT, PSEL, BAUDRATE, TASKS/EVENTS); GPIO0 @ 0x5000_0000 (OUT/OUTSET/OUTCLR/DIRSET); 1 MiB flash, 256 KiB RAM |
| nRF52840-DK board package (`configs/systems/nrf52840-dk.yaml`) | the shipped chip | the nRF52840 chip descriptor this board reuses (the silicon is identical) |

Open items: the onboard LSM9DS1 IMU is not modelled (the TWI0 controller is);
the nRF52840 USB CDC (`Serial`) is not modelled — the modelled console is
UARTE0 (`Serial1`).
