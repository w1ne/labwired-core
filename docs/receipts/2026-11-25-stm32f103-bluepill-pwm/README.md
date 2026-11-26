# STM32F103C8T6 Blue Pill: hardware PWM from stock STM32duino `analogWrite()`

**Demand rank:** STM32F103 is **#3 at 17 %** of reported MCU use (JetBrains
State of Embedded 2025). The Blue Pill is the canonical board for that part.
This is a demand pick and board-level depth on the already-modelled
STM32F103C8 — not a new chip, not a cost-ordering leftover.

**What was actually missing.** The finding was that PWM was under-tested, not
absent. The F1 fixed-map timer-output route
(`SystemBus::wire_stm32_timer_output_pads`, RM0008 §9.3.7 Tables 43-46) had no
core test — only the V2/AF-map families and the CH32V003 fixed map were
covered. One prior check existed: the Arduino matrix's L6 level counted **61
logic edges on PA0** for a NUCLEO-F103RB (2026-11-21, `min_edges: 20`), which
proves the route fires but not the frequency, duty, channel independence, or
the 0 % case; the matrix's Blue Pill row has no committed evidence directory.
The public Tier-1 grid still showed the F103 `pwm` cell as *not modelled*.
This round measures the waveform on the Blue Pill board profile with unmodified
stock firmware, adds the four missing properties as assertions, and records
what is still missing.

## Vendor status, read live 2026-11-25

- `https://www.st.com/en/microcontrollers-microprocessors/stm32f103c8.html`,
  fetched through the real Chrome (the site answers no plain HTTP client):
  page title `STM32F103C8`, status **ACTIVE**, description
  *"Mainstream Performance line, Arm Cortex-M3 MCU with 64 Kbytes of Flash
  memory, 72 MHz CPU, motor control, USB and CAN"*.
- `https://electropeak.com/stm32f103c8t6-arm-board` (same day): **In Stock**,
  SKU `MIC-02-012`, EUR 2.42, board model `STM32F103C8T6 Blue Pill ARM
  Development Board`.
- No NRND/EOL notice on either page. The board page is a retailer listing for
  a third-party minimum-system board; it does not establish authenticity of any
  individual clone.

## What boots

`tests/fixtures/stm32duino-f103-bluepill-pwm.elf` (SHA-256
`98a4277591af0f65aaa707d38f94e2f62b37cb758e6651c85ea1fb232b2bad71`) is an
unmodified STM32duino sketch — no register writes of its own:

```cpp
analogWriteFrequency(1000);
analogWrite(PA6, 64);   // TIM3_CH1 -> L298N ENA
analogWrite(PA1, 192);  // TIM2_CH2
```

Board profile: `examples/stm32f103-blue-pill/pwm-system.yaml` (STM32F103C8 at
72 MHz, USART1 console, one L298N channel with a DC-motor plant).

Measured from the captured pad waveform:

| pad | timer channel | library call | measured |
| --- | --- | --- | --- |
| PA6 | TIM3_CH1 | `analogWrite(PA6, 64)` | 1000.0 Hz at 25.10 % |
| PA1 | TIM2_CH2 | `analogWrite(PA1, 192)` | 1000.0 Hz at 75.29 % |

The two odd-looking duty numbers are the library's arithmetic, not a model
error: `HardwareTimer::setCaptureCompare(…, RESOLUTION_8B_COMPARE_FORMAT)`
programs `CCR = (ARR + 1) × value / 255`, so `analogWrite(v)` is `v/255`
(64/255 = 25.098 %, 192/255 = 75.294 %). The test asserts those exact
fractions with a ±0.001 tolerance.

Motor from the same PWM: **325.90 rpm** steady, **348.10 rpm** peak, forward,
on 12 V at a 64/255 duty.

## Evidence

Unit-level waveform (also the negative controls):

    cargo test -p labwired-core --test stm32 stm32f103_arduino_pwm:: \
        -- --test-threads=1
    # 2 passed; 0 failed
    # the same two pass on the scheduler path:
    # cargo test -p labwired-core --features event-scheduler --test stm32 \
    #     stm32f103_arduino_pwm::
    # full output: cargo-test.log

Board smoke through the same CLI path the other onboarding targets use:

    CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
      --target-id stm32f103-blue-pill \
      --prebuilt-firmware tests/fixtures/stm32duino-f103-bluepill-pwm.elf \
      --script examples/stm32f103-blue-pill/pwm.yaml \
      --system examples/stm32f103-blue-pill/pwm-system.yaml \
      --out-dir examples/stm32f103-blue-pill/evidence/pwm
    # PASS 5/5 checks (uart_ordered, two gpio_edges, motor_speed_reached,
    # expected_stop_reason), 7,305,497 steps, 10 s wall
    # committed: examples/stm32f103-blue-pill/evidence/pwm/

Rebuild the guest:

    cd tests/fixtures/stm32duino-f103-bluepill-pwm
    pio run -e bluepill
    arm-none-eabi-strip -o ../stm32duino-f103-bluepill-pwm.elf \
        .pio/build/bluepill/firmware.elf

## Controls that make the pass mean something

- **PB12**, driven high by `digitalWrite()` and owning no timer channel, shows
  **zero edges** after setup: the capture counts timer outputs, not every pad.
- **`analogWrite(PA6, 0)`** drops PA6 to a static low (zero edges over three
  1 kHz periods) while **PA1 keeps 192/255 at 1 kHz** — the channel is off
  because `CCR = 0`, not because the model stopped applying the waveform.
- **Frequency is a clock check.** At the wrong `cpu_hz` the 1 kHz call would
  land at 9 kHz (if the engine ran at 72 MHz against an 8 MHz declaration) or
  111 Hz (the reverse). The 1000.0 Hz measurement ties the board profile's
  `cpu_hz: 72_000_000` to what the firmware programs.

## Coverage and limits

Modelled and exercised here: the F1 fixed default map for TIM3_CH1 (PA6) and
TIM2_CH2 (PA1), the AF push-pull pad configuration the library writes
(`CNF = 10`, `MODE = 11`), `CCMR1` PWM mode 1 with preload, `CCER` channel
enable, `PSC`/`ARR` from `setOverflow(1000 Hz)`, `CCR` from
`setCaptureCompare`, and the live `CCR` rewrite to 0.

Not modelled, and not claimed:

- **TIM1 complementary outputs.** On this variant plain `PA7` is `TIM1_CH1N`
  in STM32duino's pin map while `TIM3_CH2` sits on `PA_7_ALT1`, and the
  library's `pwm_start()` programs channel 1 regardless of the complementary
  flag. A first build of this sketch on PA7 produced **no pad edges** in this
  engine; the complementary output stage is not modelled. Use a plain channel
  such as PA6, PA1, or PA0, as the committed sketch does.
- **AFIO remaps** (`AFIO_MAPR`): only the default fixed map is wired.
- Advanced-timer dead time, break input, and the TIM1 output stage.
- Two clocks for one board: `configs/systems/stm32f103-blue-pill.yaml` stays at
  **8 MHz HSI** because its bare-metal smoke boots from reset without the PLL
  and programs `BRR = 0x45` for that clock. This round's profile is the 72 MHz
  PLL clock that STM32duino's `SystemClock_Config()` selects. `cpu_hz` is
  static per run, so the two firmware clocks need two profiles; the PLL itself
  is a register bit in the RCC model, not a live clock tree.
- **Tier-1 grid bookkeeping.** The `pwm` cell for `stm32f103` in
  `docs/coverage/tier1-matrix.json` still reads `na`, which the scoreboard
  renders as "not modelled". The cell is derived from peripheral-id class
  markers, and the F103 descriptors name their timers `tim1..tim4` with no
  `_pwm` id; the tier-1 fixtures themselves (built from an external repository
  revision) report no `TIER1 pwm` line.
  Renaming a timer id would churn eleven descriptors and the variant-geometry
  test for a reporting-only change, so it is left as a known reporting gap
  rather than a silent claim. The capability itself is proven above.
