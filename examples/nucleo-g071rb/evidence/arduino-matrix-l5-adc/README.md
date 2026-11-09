# Arduino matrix L5 (stock `analogRead(A0)`) — STM32G0 ADC

Date: 2026-11-09. Measured on this branch (cut from `main`), release CLI.

## Result

Stock STM32duino `analogRead(A0)` now converts the voltage held on A0
(PA0 = ADC1_IN0). The matrix system holds 1651 mV; 12 bits against 3.3 V is
2048, and the core returns 10 bits.

| Board | Cell | Measured | Expected |
|---|---|---|---|
| NUCLEO-G071RB | `stm32g071rb × L5_adc` | `LW_L5_VAL 512 512` | 512 |
| NUCLEO-G0B1RE | `stm32g0b1re × L5_adc` | `LW_L5_VAL 512 512` | 512 |

Evidence: `pass/` here and in
`examples/nucleo-g0b1re/evidence/arduino-matrix-l5-adc/pass/`.

## Before this change

The G0 ADC used the `stm32l4` profile. The G071 L5 cell printed `LW_L5_OK`
with `LW_L5_VAL 0 0` (measured on the lane build): no conversion, but the
generic oracle accepted any equal pair. The G071 and G0B1 L5 passes were
vacuous.

## Controls (same G071 firmware, system changed)

| Control | Measured | Verdict |
|---|---|---|
| A0 not held (`control-unheld-a0/`) | `LW_L5_VAL 930 930` | `LW_L5_FAIL` |
| A0 held at 2000 mV (`control-a0-2000mv/`) | `LW_L5_VAL 620 620` | `LW_L5_FAIL` |

930 is the model's fixed 3.0 V fallback source at 10 bits. 620 is 2000 mV
at 10 bits. The code follows the held input.

## Full rows

`rows-g071-g0b1-ststm32-19.7.1.md`: 17 pass / 1 skip / 0 fail (G071 L8 has
no CAN). Built with ST STM32 19.7.1, the CI pin. With the local 20.0.0, L4
fails on both rows because its `SPI.begin()` is a no-op; that is not this
change.

## Model limits (`stm32g0` ADC profile)

One conversion per start: a multi-channel CHSELR group stops after its
first channel and does not set EOS. No conversion IRQ, DMA request,
overrun, analog watchdog, or internal channel (VREFINT, TS, VBAT).

## Vendor status (read live in Chrome, 2026-11-09)

- https://www.st.com/en/microcontrollers-microprocessors/stm32g071rb.html — ACTIVE
- https://www.st.com/en/microcontrollers-microprocessors/stm32g0b1re.html — ACTIVE
- https://www.st.com/en/evaluation-tools/nucleo-g071rb.html — ACTIVE
