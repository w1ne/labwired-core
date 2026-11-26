# STM32F103C8T6 Blue Pill

**Demand position:** STM32F103 ranks #3 in the measured MCU list (17%). This
adds a board-level profile for the popular Blue Pill using the existing
STM32F103C8 model; it does not add a new chip family.

Two smokes cover this profile. The GPIO smoke boots a bare-metal Cortex-M3
image, sends a marker through USART1, and toggles the board's active-low PC13
user LED. The PWM smoke boots stock STM32duino and drives TIM3_CH1 (PA6) and
TIM2_CH2 (PA1). The profile does not model the USB device, USB bootloader,
onboard debugger, TIM1 complementary outputs, or every Blue Pill clone's board
differences.

## Pin map

| Board signal | MCU pin | Notes |
| --- | --- | --- |
| User LED | PC13 | Active low on the reference schematic |
| USART1 TX | PA9 | Header pin; connect to a 3.3 V adapter's RX |
| USART1 RX | PA10 | Header pin; connect to adapter TX if needed |
| SWDIO / SWCLK | PA13 / PA14 | External SWD programmer; not exercised here |
| PWM PA6 | PA6 | TIM3_CH1; L298N ENA in the PWM smoke |
| PWM PA1 | PA1 | TIM2_CH2; second independent channel in the PWM smoke |

The board has no USB-UART bridge. For a physical serial view, connect a 3.3 V
USB-serial adapter to PA9 and GND. The smoke's captured UART is a simulator
console route, not a simulated USB bridge.

## Copy-paste Arduino example

Select the STM32F103C8T6 Blue Pill board in STM32duino and use USART1 on PA9:

```cpp
#include <Arduino.h>

void setup() {
  pinMode(PC13, OUTPUT);
  Serial1.begin(115200);  // USART1 TX=PA9, RX=PA10
  Serial1.println("BLUEPILL_F103C8_OK");
}

void loop() {
  digitalWrite(PC13, LOW);   // active-low user LED on
  delay(500);
  digitalWrite(PC13, HIGH);  // user LED off
  delay(500);
}
```

## Hardware PWM: stock `analogWrite()` driving an L298N

The second smoke runs an unmodified STM32duino sketch at the board's PLL clock
(72 MHz) and measures the two channels on the pads:

| pad | channel | call | measured |
| --- | --- | --- | --- |
| PA6 | TIM3_CH1 | `analogWrite(PA6, 64)` | 1000.0 Hz at 25.10 % (64/255) |
| PA1 | TIM2_CH2 | `analogWrite(PA1, 192)` | 1000.0 Hz at 75.29 % (192/255) |

`analogWrite(v)` is `v/255`, not `v/256`: the library's
`setCaptureCompare(…, RESOLUTION_8B_COMPARE_FORMAT)` programs
`CCR = (ARR + 1) × v / 255`. The committed sketch:

```cpp
#include <Arduino.h>

static void report(const char *tag, int value) {
  Serial.print(tag);
  Serial.print(' ');
  Serial.println(value);
}

void setup() {
  Serial.begin(115200);

  // L298N channel A: IN1 high, IN2 low is "forward".
  pinMode(PB12, OUTPUT);
  pinMode(PB13, OUTPUT);
  digitalWrite(PB12, HIGH);
  digitalWrite(PB13, LOW);

  // 1 kHz carrier, then 64/255 on PA6 and 192/255 on PA1 (8-bit resolution).
  analogWriteFrequency(1000);
  analogWrite(PA6, 64);
  analogWrite(PA1, 192);

  Serial.println("F103PWM READY");
  report("PA6_DUTY", 64);
  report("PA1_DUTY", 192);
}

void loop() {
  delay(100);

  // The same channel at 0 %: the pin must go low and stay low.
  analogWrite(PA6, 0);
  Serial.println("F103PWM STOP");
  report("PA6_DUTY", 0);

  for (;;) {
  }
}
```

With a 12 V supply and the same motor plant as
[`f401-hbridge-dc-motor`](../f401-hbridge-dc-motor/README.md), the 64/255 duty
settles at **325.90 rpm** (348.10 rpm peak, forward), and the run passes five
checks: the UART order, an edge count per pad, the motor speed, and the stop
reason.

Reproduce it (the guest is a PlatformIO build, so it is passed as a prebuilt
ELF):

```sh
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id stm32f103-blue-pill \
  --prebuilt-firmware tests/fixtures/stm32duino-f103-bluepill-pwm.elf \
  --script examples/stm32f103-blue-pill/pwm.yaml \
  --system examples/stm32f103-blue-pill/pwm-system.yaml \
  --out-dir examples/stm32f103-blue-pill/evidence/pwm
```

Rebuild the guest with `pio run -e bluepill` in
`tests/fixtures/stm32duino-f103-bluepill-pwm`, then
`arm-none-eabi-strip` the `firmware.elf`. Evidence is committed under
[`evidence/pwm/`](evidence/pwm/), and the waveform-level test with its
negative controls is
`crates/core/tests/stm32/stm32f103_arduino_pwm.rs`.

**Clock note.** `pwm-system.yaml` runs at 72 MHz because that is what stock
STM32duino selects; the board profile `configs/systems/stm32f103-blue-pill.yaml`
stays at the 8 MHz HSI reset clock its bare-metal smoke boots at and programs
`BRR` for. `cpu_hz` is static per run in this engine, so the two firmware
clocks are two profiles.

**Not claimed.** TIM1 complementary outputs (`analogWrite(PA7, …)` produces no
waveform here), the AFIO remaps, dead time/break, the USB device, the
bootloader, and physical-board validation.

## Reproduce the committed local simulator smoke

```sh
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id stm32f103-blue-pill \
  --crate firmware-stm32f103-bluepill-smoke \
  --target thumbv7m-none-eabi \
  --script examples/stm32f103-blue-pill/io-smoke.yaml \
  --system configs/systems/stm32f103-blue-pill.yaml \
  --out-dir examples/stm32f103-blue-pill/evidence
```

The smoke asserts the USART1 marker, at least four PC13 transitions, and a
bounded stop at the configured step limit. Evidence and the onboarding metric
are committed under `evidence/`. The board is not hardware-confirmed and is
not a hosted LabWired `model_verified` target.

See [`vendor-status.md`](vendor-status.md) for the live lifecycle and board
availability check.
