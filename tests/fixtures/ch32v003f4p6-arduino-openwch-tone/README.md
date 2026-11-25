# CH32V003F4P6 openwch Arduino `tone()` fixture

Built exactly as `../ch32v003f4p6-arduino-openwch/` (PlatformIO `ch32v`
platform 1.1.0, `board = ch32v003f4p6_evt_r0`, `framework = arduino`,
`board_build.core = openwch`, `framework-arduino-openwch-ch32` at
`3820604346d0a9f0ee9a72cab8b7583e488e6674`). The sketch is not modified for
the simulator.

What the sketch exercises:

- `tone(PD4, 1000)`. On this variant the core's `Tone.cpp` sets
  `TIMER_TONE = TIM2`, computes `ARR` from the timer clock to
  `48 MHz / (2 * 1000) = 24 000` counts, attaches a callback that toggles PD4
  and enables `TIM2_IRQn` (38). The pad waveform is produced only by the
  update interrupt; the sketch never touches PD4 after `tone()` returns.
- the default (non-remapped) channel map: PD4 = TIM2_CH1. `tone()` drives the
  pad as a plain GPIO output, not through the timer channel.
- the sketch prints `CH32V003_TONE_BOOT`, `CH32V003_TONE_SET` and
  `CH32V003_TONE_DONE` on USART1 (PD5), so the test can tell setup finished
  before it measures the steady-state waveform.

Rebuild:

    cd tests/fixtures/ch32v003f4p6-arduino-openwch-tone
    pio run
    riscv-wch-elf-objcopy --strip-debug .pio/build/sim/firmware.elf \
        ../ch32v003f4p6-arduino-openwch-tone.elf

Test: `crates/core/tests/vendor/ch32v003_compiled_firmware.rs`
(`ch32v003_openwch_tone_toggles_pd4_from_the_tim2_update_isr` and
`ch32v003_tone_needs_the_tim2_irq_line`). The simulator measures the pad from
logic edges: eight edges, constant 24 072-cycle toggle spacing. The spacing is
the programmed 24 000-count half period plus the 72 cycles the ISR takes to
clear `UIF` (the timer model holds the counter while an enabled flag is
latched), so the square wave is 997.0 Hz rather than exactly 1 kHz. The
negative control removes the TIM2 `irq` line: the sketch still reaches
`TONE_DONE`, but the pad stays flat.
