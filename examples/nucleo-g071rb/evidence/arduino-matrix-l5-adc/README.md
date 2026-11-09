# Arduino matrix L5 (stock `analogRead(A0)`) — STM32G0/C0 ADC

Date: 2026-11-09. Branch: `lane/new-hw-onboarding`.

## Result

Stock STM32duino `analogRead(A0)` now converts the voltage held on A0
(PA0 = ADC1_IN0) on three Nucleo boards. The matrix system holds 1651 mV;
12 bits against 3.3 V is 2048, and the core returns 10 bits.

| Board | Cell | Measured | Expected |
|---|---|---|---|
| NUCLEO-G071RB | `stm32g071rb × L5_adc` | `LW_L5_VAL 512 512` | 512 |
| NUCLEO-G0B1RE | `stm32g0b1re × L5_adc` | `LW_L5_VAL 512 512` | 512 |
| NUCLEO-C031C6 | `stm32c031c6 × L5_adc` | `LW_L5_VAL 512 512` | 512 |

Evidence: `pass/` in this directory and in
`examples/nucleo-g0b1re/evidence/arduino-matrix-l5-adc/` and
`examples/nucleo-c031c6/evidence/arduino-matrix-l5-adc/`.

## Before this change

The G0/C0 ADC used the `stm32l4` profile. On the same G071 firmware it
printed `LW_L5_OK` with `LW_L5_VAL 0 0`: no conversion, but the generic
oracle accepted any equal pair. The G071 and G0B1 L5 passes were vacuous,
and the C031 row skipped L5.

## Controls (same G071 firmware, system changed)

| Control | Measured | Verdict |
|---|---|---|
| A0 not held (`control-unheld-a0/`) | `LW_L5_VAL 930 930` | `LW_L5_FAIL` |
| A0 held at 2000 mV (`control-a0-2000mv/`) | `LW_L5_VAL 620 620` | `LW_L5_FAIL` |

930 is the model's fixed 3.0 V fallback source at 10 bits. 620 is
2000 mV at 10 bits. The code follows the held input.

## Model limits (`stm32g0` ADC profile)

One conversion per start: a multi-channel CHSELR group stops after its
first channel and does not set EOS. No conversion IRQ, DMA request,
overrun, analog watchdog, or internal channel (VREFINT, TS, VBAT).

## Vendor status (read live in Chrome, 2026-11-09)

- https://www.st.com/en/microcontrollers-microprocessors/stm32g071rb.html — ACTIVE
- https://www.st.com/en/microcontrollers-microprocessors/stm32g0b1re.html — ACTIVE
- https://www.st.com/en/evaluation-tools/nucleo-g071rb.html — ACTIVE
- STM32C031C6: ACTIVE, recorded in `examples/nucleo-c031c6/evidence/vendor-status.md`.

## Full rows (2026-11-09)

`rows-local-ststm32-20.0.0.md`: G071, G0B1 and C031 rows, 22 pass / 3 skip /
2 fail. The two fails are L4 on G071 and G0B1. Those rows do not pin the
platform, so the local build used ST STM32 20.0.0, whose `SPI.begin()` is a
no-op. CI pins 19.7.1. `rows-g071-g0b1-L4-L5-ststm32-19.7.1.md`: the same L4
and L5 cells on 19.7.1, 4 pass / 0 fail.
