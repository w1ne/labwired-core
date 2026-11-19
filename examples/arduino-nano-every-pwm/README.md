# Arduino Nano Every PWM (`analogWrite`)

Run commands from the repository root.

Demand position: Arduino kits rank 3 at 37% in the JetBrains State of Embedded
survey. The ATmega4809 is not ranked separately. The survey's #2 MCU is the
ATmega328, which Microchip marks Not Recommended for New Designs; the Nano
Every is the in-production Arduino board in the same form factor. Live stock
evidence is in [`vendor-status.md`](vendor-status.md). This is peripheral
depth on a chip that was already onboarded, not a new chip.

Stock `analogWrite()` now puts a waveform on the pad. The Nano Every has two
timer kinds behind it, and this example drives one pin of each.

## Pins

No part is needed beyond an Arduino Nano Every (ABX00028). Put an LED with a
series resistor, or a scope probe, on D9 and on D3.

| Arduino pin | Pad | Timer output |
| --- | --- | --- |
| D9 | PB0 | TCA0 WO0 |
| D10 | PB1 | TCA0 WO1 |
| D5 | PB2 | TCA0 WO2 |
| D6 | PF4 | TCB0 WO (alternate pad) |
| D3 | PF5 | TCB1 WO (alternate pad) |

ArduinoCore-megaavr runs TCA0 single-slope at 16 MHz / 64 with PER = 255 and
each TCB in 8-bit PWM mode on the same clock. Both give 16 MHz / 64 / 256 =
976.6 Hz, and `analogWrite(pin, n)` is high for `n` of 256 timer ticks.

## Copy-paste Arduino example

Select **Arduino Nano Every** and upload:

```cpp
#include <Arduino.h>

// D9 = PB0 = TCA0 WO0, D3 = PF5 = TCB1 WO: the two timer kinds behind
// analogWrite() on the Nano Every.
static const uint8_t kTcaPin = 9;
static const uint8_t kTcbPin = 3;

void setup() {
  Serial.begin(115200);
  analogWrite(kTcaPin, 64);   // 25 % duty
  analogWrite(kTcbPin, 192);  // 75 % duty
  Serial.println("NANO_EVERY_PWM");
  delay(20);
  analogWrite(kTcaPin, 128);  // 50 % duty, taken at the next period
  Serial.println("PWM_HALF");
  delay(20);
  digitalWrite(kTcbPin, LOW);  // stops the PWM on D3
  Serial.println("PWM_D3_OFF");
}

void loop() {}
```

The exact sketch is [`firmware/src/main.cpp`](firmware/src/main.cpp). The
committed build is `tests/fixtures/avr/arduino-nano-every-pwm.elf`;
`firmware/build.sh` rebuilds it and copies it there.

## Build and simulate

```bash
python -m pip install platformio
examples/arduino-nano-every-pwm/firmware/build.sh
cargo run -q -p labwired-cli -- test \
  --script examples/arduino-nano-every-pwm/pwm-smoke.yaml --no-uart-stdout
cargo test -p labwired-core --test avr avr_nano_every_pwm::
```

The Rust test runs on pull requests from the committed ELF. It needs no
PlatformIO. It measures, on the pads, in CPU cycles at 16 MHz:

| Pad | Call | Period | High time |
| --- | --- | --- | --- |
| D9 (PB0) | `analogWrite(9, 64)` | 16384 | 4096 |
| D9 (PB0) | `analogWrite(9, 128)` | 16384 | 8192 |
| D3 (PF5) | `analogWrite(3, 192)` | 16384 | 12288 |

It also requires that D3 has no edge after `digitalWrite(3, LOW)` and ends
low, and that D10 (PB1), which the sketch never drives, has no edge.

## Evidence

| Run | Result | Record |
| --- | --- | --- |
| `pwm-smoke.yaml` | PASS, 6/6 checks; 473 edges on PB0 and 80 on PF5 in 2 000 000 steps | [`evidence/run/`](evidence/run/) |
| `avr_nano_every_pwm` Rust test, without and with the event scheduler | PASS, exact period and high times | `cargo test -p labwired-core --test avr avr_nano_every_pwm::` |
| Control, edges required on D10 (PB1) ([`controls/other-pad.yaml`](controls/other-pad.yaml)) | FAIL as required, 0 edges | [`evidence/controls/other-pad/`](evidence/controls/other-pad/) |
| Control, 200 edges required on D3 ([`controls/d3-keeps-running.yaml`](controls/d3-keeps-running.yaml)) | FAIL as required, the waveform stops at `digitalWrite(3, LOW)` after 80 edges | [`evidence/controls/d3-keeps-running/`](evidence/controls/d3-keeps-running/) |
| Control, timer outputs not wired to the ports | Rust test FAILS as required, `TCA0 compare outputs (CTRLB.CMPnEN) with no port to carry them` at the first `analogWrite()` | [`evidence/controls/no-output-wiring/`](evidence/controls/no-output-wiring/) |
| Arduino matrix cell `L6_pwm` | PASS, 60 edges on PB0 (20 required); row 8 pass / 1 skip / 0 fail | [`../arduino-nano-every/evidence/arduino-matrix/L6_pwm/`](../arduino-nano-every/evidence/arduino-matrix/L6_pwm/) |

This is repository smoke evidence. It is not a `labwired_verify`
model-verification verdict: the hosted verifier catalog has no ATmega4809
target. No physical board test was run.

## Model boundary

Modelled and exercised: TCA0 single-slope compare outputs (`CTRLB.CMPnEN`),
high while CNT < CMPn; the CMPnBUF update at the period boundary; the
`PORTMUX.TCAROUTEA` route to PORTB; TCB 8-bit PWM output (`CTRLB.CCMPEN`),
high while CNT < CCMPH; the `PORTMUX.TCBROUTEA` alternate pad; the PORTx.DIR
condition; the return of the pad to PORTx.OUT when the output is disabled.

Modelled and unit-tested only: the other five TCA0 routes, the TCB default
pads, CMPn = 0 (always low) and CMPn > PER (always high), the level of a
stopped timer (`CTRLC.CMPnOV`, `CTRLB.CCMPINIT`).

Refused with an error, not ignored: TCA0 split mode, FRQ and the dual-slope
modes, TCA0 interrupts and CTRLESET commands, a compare output in NORMAL
mode, the TCB count modes other than INT and PWM8, the event inputs.

Not modelled: pad voltage, drive strength and slew. An edge is published at
the CPU cycle of its match or wrap; a pulse shorter than one CPU instruction
can merge with its neighbour. A `PORTMUX` route write on its own moves a
running waveform at its next edge, not at once. After an output is disabled
the pad level is correct, but a four-state trace shows the pad as released
until PORTx next moves it. The run assumes an ideal 5 V rail.
