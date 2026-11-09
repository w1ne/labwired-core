# Arduino matrix L5 (stock `analogRead(A0)`) — STM32L0 ADC

Date: 2026-11-09. Branch: `lane/new-hw-onboarding`.

## Result

Stock STM32duino `analogRead(A0)` on the NUCLEO-L073RZ now converts the
voltage held on A0 (PA0 = ADC_IN0). The matrix system holds 1651 mV; 12 bits
against 3.3 V is 2048.

| Core | Cell | Measured | Expected |
|---|---|---|---|
| stm32duino 2.12.0 (ststm32 19.7.1, CI pin) | `stm32l073 × L5_adc` | `LW_L5_VAL 512 512` | 512 |
| stm32duino 3.0.0 (ststm32 20.0.0) | `stm32l073 × L5_adc` | `LW_L5_VAL 511 511` | 511 |

The two cores read differently on the L0, and both values are exact:

* Core 2.12.0 picks the internal read resolution from HAL `ADC_RESOLUTION_10B`
  and converts at 10 bits: 512.
* Core 3.0.0 keys on `LL_ADC_RESOLUTION_10B`, which `wiring_analog.c` does not
  see on the L0. The firmware's `_internalReadResolution` in `.data` is `0x0C`
  (12). It converts at 12 bits and `mapResolution()` returns
  `((2048 + 1) >> 2) - 1 = 511`. Only codes 2047..2050 give 511.

The L5 sketch selects the expected value with `STM32_CORE_VERSION_MAJOR`.

Full row on the CI pin (`row-stm32l073-ststm32-19.7.1.log`):
8 pass / 1 skip (L8, no CAN on L0) / 0 fail. Before: 7 pass / 2 skip.

## Before this change

The L073 ADC used the `stm32l4` profile, which reads the regular sequence
from SQR1. The L0 has no SQR1; its sequence is the `CHSELR` bitmap at 0x28.
On the same firmware the old profile printed `LW_L5_VAL 0 0`
(`control-old-stm32l4-profile/`), and the row skipped L5.

## Controls (stm32duino 3.0.0 firmware, system changed)

| Control | Measured | Verdict |
|---|---|---|
| Old `stm32l4` profile | `0 0` | FAIL (no conversion) |
| A0 not held (fixed 3.0 V source) | `930 930` | FAIL |
| A0 held at 2000 mV | `619 619` | FAIL; 12-bit 2482, `((2482 + 1) >> 2) - 1 = 619` |

## Model change

`stm32l0` / `l0` is a new name for the existing `Stm32G0` ADC layout. The L0
map (RM0367 §14.12, `configs/peripherals/stm32l073/adc.yaml`) is a subset of
the G0 map at the same offsets. Limits are those of the G0 layout: one
conversion per start; no IRQ, DMA, overrun, analog watchdog or internal
channel (VREFINT, TS). The G0 layout answers 0x0FFF_0000 at 0x24 and 0x2C,
which are reserved on the L0.

## Vendor status

Read live 2026-11-09 on st.com (Chrome tab):
`https://www.st.com/en/microcontrollers-microprocessors/stm32l073rz.html` → ACTIVE;
`https://www.st.com/en/evaluation-tools/nucleo-l073rz.html` → ACTIVE.

The `test.yaml` files name the `/tmp` paths the runs used. Rerun with
`/usr/bin/python3 validation/arduino-matrix/run_matrix.py --boards stm32l073 --sketches L5_adc`.
