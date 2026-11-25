# CH32V003F4P6 openwch Arduino `analogWrite` fixture

Built exactly as `../ch32v003f4p6-arduino-openwch/` (PlatformIO `ch32v`
platform 1.1.0, `board = ch32v003f4p6_evt_r0`, `framework = arduino`,
`board_build.core = openwch`, `framework-arduino-openwch-ch32` at
`3820604346d0a9f0ee9a72cab8b7583e488e6674`). The sketch is not modified for
the simulator.

What the sketch exercises:

- `analogWrite(PD4, 2048)` and `analogWrite(PD2, 1024)` on the core's default
  1000 Hz / 12-bit PWM settings. `pwm_start()` enables TIM2/TIM1 in RCC,
  configures PWM mode 1 with `OCxPE`, writes `PSC`/`ATRLR`/`CHxCVR`, sets
  `BDTR.MOE` and starts the counter; `pinmap_pinout()` switches the pad to
  alternate-function push-pull (`CFGLR` nibble 0b1011);
- the default (non-remapped) pads: PD4 = TIM2_CH1, PD2 = TIM1_CH1;
- the sketch prints `CH32V003_PWM_BOOT`, `CH32V003_PWM_SET` and
  `CH32V003_PWM_DONE` on USART1 (PD5), so the test can tell setup finished
  before it measures the steady-state waveform.

Rebuild:

    cd tests/fixtures/ch32v003f4p6-arduino-openwch-pwm
    pio run
    riscv-wch-elf-objcopy --strip-debug .pio/build/sim/firmware.elf \
        ../ch32v003f4p6-arduino-openwch-pwm.elf

Test: `crates/core/tests/vendor/ch32v003_compiled_firmware.rs`
(`ch32v003_openwch_analog_write_pwms_tim2_ch1_and_tim1_ch1` and the
unclocked-TIM2 negative control). The simulator measures the pads from logic
edges, not from timer registers: period 48 000 cycles (1 kHz at 48 MHz HCLK),
high time 24 005 (PD4, 2048/4095) and 12 002 (PD2, 1024/4095).
