# Teensy 4.1 — stock Teensyduino matrix row

Stock Teensyduino (PlatformIO `teensy@6.0.0`, framework-arduinoteensy
1.162.0 = Teensyduino 1.62, board `teensy41`), the matrix sketches unchanged
except for the L5 oracle branch, run by
`validation/arduino-matrix/run_matrix.py --boards teensy41` on
`configs/chips/mimxrt1062-teensy41.yaml` (system
`validation/arduino-matrix/systems/teensy41.yaml`).

Board: PJRC Teensy 4.1, MIMXRT1062DVJ6B. SparkFun (which manufactures Teensy
now) lists DEV-16771 in stock at $31.50 with no retirement notice, read live
2026-11-08 (https://www.sparkfun.com/teensy-4-1.html); pjrc.com shows no
discontinued notice for the 4.1.

| cell | result | what it proves |
|---|---|---|
| L0 serial boot | pass | FlexSPI NOR IVT boot, ResetHandler (FlexRAM, ITCM/DTCM copy, VTOR, PLLs/DCDC via `set_arm_clock`, cache/MPU setup, PSRAM probe), SysTick on the 100 kHz reference, USB1 enumeration + CDC open by the scripted host; `Serial` = USB CDC |
| L1 serial loop | pass | `loop()` + `delay()`/`millis()` on SysTick |
| L2 blink | pass, 3 edges | `digitalWrite(13)` toggles GPIO_B0_03 via fast GPIO7 (`led_watch: gpio7:3`) |
| L3 I2C | pass, exact tier | `Wire` = LPI2C1 reads INA219 config 0x399F and bus 0x19CA |
| L4 SPI | skip | no LPSPI model: SPI = LPSPI4 is unmapped |
| L5 ADC | pass, exact | `analogRead(A0)` = 512 / 512 with A0 (ADC1_IN7) held at 1650 mV, 10-bit |
| L6 PWM | skip (measured fail) | `analogWrite(13)` is QuadTimer2, a declared stub: 0 pad edges with `pwm_watch: gpio7:3` (`L6_pwm/result.json`). The UART marker alone would have "passed"; it is not counted |
| L7 timer | pass | `micros()` (DWT CYCCNT + SysTick) advances monotonically across `delay(2)` |
| L8 CAN | skip | no FlexCAN model |

Each cell directory holds the matrix's `test.yaml`, `result.json`,
`uart.log` and `junit.xml` as `run_matrix.py` ran them (they stop at the
first assertion pass). Paths are made repo-relative.

## Timing is real, so the row is slow

Teensyduino's startup busy-waits on `millis()` for 20 ms before `usb_init()`
and 280 ms after it, so `setup()` starts at 300 ms = 180M cycles at 600 MHz.
L0 passes at 180,658,838 steps. The row therefore sets `max_steps: 250M` with
a `wall_time_ms` bound (the CLI refuses more than 50M steps without one).

## Negative control (`L0_serial_boot/controls/`)

Same L0 ELF, same system, on a chip copy whose SysTick declares no
`external_ref_hz` (the engine's previous behaviour: SysTick counts CPU cycles
whatever CLKSOURCE says). `millis()` then runs 6000x fast: the 300 ms wait
ends in ~30k cycles, `setup()` prints before the host has configured the
device, Teensyduino drops the write (`usb_configuration == 0`), and the run
ends at `max_steps` (250M) with an empty `uart.log`.

## What is modelled and what is not

See the header of `configs/chips/mimxrt1062-teensy41.yaml`. Stubbed (reads
0): IOMUXC_GPR, IOMUXC_SNVS(_GPR), FlexRAM, SNVS, PIT, OCOTP, SRC, GPC,
QuadTimer1-4, eFlexPWM1-4, and the CoreDebug block at 0xE000_EDF0
(ResetHandler2 sets DEMCR.TRCENA; main maps the real block only under an
SWD attach, so without the stub that store is a BusFault). FlexRAM is a fixed 512 KB ITCM + 512 KB DTCM, not
the GPR17 bank split. Unmapped (faults): LPSPI, FlexCAN, eDMA, ENET, SDHC,
SAI, the second USB port and everything else not listed.
