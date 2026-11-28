# CH32V003F4P6 openwch Arduino `analogRead` fixture

Built exactly as `../ch32v003f4p6-arduino-openwch/` (PlatformIO `ch32v`
platform 1.1.0, `board = ch32v003f4p6_evt_r0`, `framework = arduino`,
`board_build.core = openwch`, `framework-arduino-openwch-ch32` at
`3820604346d0a9f0ee9a72cab8b7583e488e6674`). The sketch is not modified for
the simulator.

What the sketch exercises:

- `analogRead(PA2)` (ADC1_IN0) and `analogRead(PD4)` (ADC1_IN7). The core's
  `adc_read_value` enables `RCC_ADC1EN` (APB2PCENR bit 9), runs `ADC_Init`,
  clears STATR, sets ADON, writes RSQR3 rank 1 and SAMPTR, sets
  CTLR2.SWSTART, polls STATR.EOC and returns RDATAR. The V00x build defines
  no `ADC_CTLR_ADCAL`, so no calibration runs;
- the variant sets `ADC_RESOLUTION 10`, so `analogRead` returns RDATAR
  unchanged (0..1023).

The sketch prints `pa2=<n>` and `pd4=<n>`, then `CH32V003_ADC_PASS` only when
both values are 10-bit codes and PA2 reads lower than PD4.

Rebuild:

    cd tests/fixtures/ch32v003f4p6-arduino-openwch-adc
    pio run
    riscv-wch-elf-objcopy --strip-debug .pio/build/sim/firmware.elf \
        ../ch32v003f4p6-arduino-openwch-adc.elf

Test: `crates/core/tests/vendor/ch32v003_compiled_firmware.rs`
(`ch32v003_openwch_analog_read_returns_10_bit_codes` and the negative
control `ch32v003_analog_read_overflows_10_bits_with_the_stm32f1_adc`).
