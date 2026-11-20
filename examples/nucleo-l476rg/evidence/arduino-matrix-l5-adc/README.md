# Arduino matrix L5 (stock `analogRead(A0)`) — STM32 L4/G4/WB ADC

Date: 2026-11-09. Branch: `lane/new-hw-onboarding`.

## Result

Stock STM32duino `analogRead(A0)` now converts the voltage held on A0 on
three Nucleo boards that use the `stm32l4` ADC profile. The matrix system
holds 1651 mV; 12 bits against 3.3 V is 2048, and the core returns 10 bits.

| Board | A0 | Cell | Measured | Expected |
|---|---|---|---|---|
| NUCLEO-L476RG | PA0 = ADC1_IN5 | `stm32l476 × L5_adc` | `LW_L5_VAL 512 512` | 512 |
| NUCLEO-G474RE | PA0 = ADC1_IN1 | `stm32g474re × L5_adc` | `LW_L5_VAL 512 512` | 512 |
| P-NUCLEO-WB55 | PC0 = ADC1_IN1 | `stm32wb55 × L5_adc` | `LW_L5_VAL 512 512` | 512 |

Evidence: `pass/` in this directory and in
`examples/nucleo-g474re/evidence/arduino-matrix-l5-adc/` and
`examples/nucleo-wb55rg/evidence/arduino-matrix-l5-adc/`. `pass/uart.log`
is the matrix run (it stops when `LW_L5_OK` appears); `pass/uart-full.log`
is the same firmware run to the step limit, with the `LW_L5_VAL` line.
`result.json` files omit the `inspect` memory dump.

## Before this change

On the same firmware all three boards printed `LW_L5_OK` with
`LW_L5_VAL 0 0`. No conversion occurred, but the generic oracle accepted any
equal pair, so the three L5 passes were vacuous. Two model defects caused it:

1. `CR.ADCAL` never self-cleared on the `stm32l4` layout. stm32duino selects
   `ADC_CLOCK_SYNC_PCLK_DIV4` (`CCR.CKMODE = 0b11`), so silicon has a clock
   and calibration completes. `HAL_ADCEx_Calibration_Start` timed out.
2. `CR` stored written zeros in its rs bits. `LL_ADC_REG_StartConversion`
   writes `MODIFY_REG(CR, ADC_CR_BITS_PROPERTY_RS, ADSTART)`, which has
   ADEN = 0. The model cleared ADEN and did not convert. ADSTP and ADDIS
   also did not complete, so `HAL_ADC_Stop` failed.

The engine also ignored `SQR1.SQ1` and converted a fixed source.

## Controls (same L476 firmware, system changed)

| Control | Measured | Verdict |
|---|---|---|
| A0 not held (`control-unheld-a0/`) | `LW_L5_VAL 930 930` | `LW_L5_FAIL` |
| A0 held at 2000 mV (`control-a0-2000mv/`) | `LW_L5_VAL 620 620` | `LW_L5_FAIL` |

930 is the model's fixed 3.0 V fallback source at 10 bits. 620 is 2000 mV
at 10 bits. The code follows the held input on the SQ1 channel.

The silicon capture `firmware_survival::nucleo_l476rg_adc` (ADCAL stays set
with no ADC clock: CCIPR and CKMODE untouched) still passes.

## Model limits (`stm32l4` ADC profile)

Right alignment only. One conversion per start: ranks after SQ1 are not
converted. No conversion IRQ, DMA request, overrun, oversampling, offset,
analog watchdog, or internal channel (VREFINT, TS, VBAT). With CKMODE = 0
calibration completes only on layouts that declare a kernel clock; the RCC
`CCIPR.ADCSEL` path is not modelled.

NUCLEO-L073RZ also uses this profile, but its ADC is the L0 CHSELR block.
Its L5 still reads `0 0`, so the matrix now skips that cell (see
`boards.yaml`); open PR #1722 adds the F0/L0 model.

## Vendor status (read live in Chrome, 2026-11-09)

- https://www.st.com/en/microcontrollers-microprocessors/stm32l476rg.html — ACTIVE
- https://www.st.com/en/microcontrollers-microprocessors/stm32g474re.html — ACTIVE
- https://www.st.com/en/microcontrollers-microprocessors/stm32wb55rg.html — ACTIVE
- https://www.st.com/en/evaluation-tools/nucleo-l476rg.html — ACTIVE
- https://www.st.com/en/evaluation-tools/nucleo-g474re.html — ACTIVE
- https://www.st.com/en/evaluation-tools/p-nucleo-wb55.html — ACTIVE

## Full rows (2026-11-09)

`rows-local-ststm32-20.0.0.md`: G474, L073, L476 and WB55 rows, 29 pass /
3 skip / 4 fail. This run predates the L073 L5 skip; its L073 L5 "pass" is
the vacuous `0 0`. The four fails are L4 SPI: those rows do not pin the
platform, so the local build used ST STM32 20.0.0, whose `SPI.begin()` is a
no-op. CI pins 19.7.1. `rows-L4-L5-ststm32-19.7.1.md`: L4 and L5 on the three
L4-profile boards on 19.7.1, 6 pass / 0 fail.
