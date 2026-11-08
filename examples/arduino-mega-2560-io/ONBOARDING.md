# Arduino Mega 2560 Rev3 onboarding

The Arduino Mega 2560 Rev3 is orderable from Arduino, and Microchip reports the
ATmega2560 as **In Production**. The live checks and URLs are recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Modeled board path

- Board system: [`configs/systems/arduino-mega-2560.yaml`](../../configs/systems/arduino-mega-2560.yaml)
- MCU profile: [`configs/chips/atmega2560.yaml`](../../configs/chips/atmega2560.yaml)
- D13 / `LED_BUILTIN` maps to PB7 (`portb:7`), active high.
- `Serial` uses USART0, connected to the host console. The board's ATmega16U2
  USB-to-serial bridge and USB enumeration are outside this model.
- The example system adds centered 5 V potentiometers on A8/ADC8 and A9/ADC9.

Rebuild the Arduino-core firmware and reproduce the committed simulator evidence:

```bash
scripts/onboarding_smoke.sh \
  --target-id arduino-mega-2560 \
  --firmware-build-script examples/arduino-mega-2560-io/ci/build-firmware.sh \
  --script examples/arduino-mega-2560-io/io-smoke.yaml \
  --system examples/arduino-mega-2560-io/system.yaml \
  --out-dir examples/arduino-mega-2560-io/evidence

python3 scripts/generate_onboarding_scoreboard.py \
  --metrics-root examples/arduino-mega-2560-io/evidence \
  --markdown-out examples/arduino-mega-2560-io/evidence/onboarding-scoreboard.md \
  --json-out examples/arduino-mega-2560-io/evidence/onboarding-scoreboard.json
```

The smoke asserts the USART0 banner, the ADC readings from A8/A9/A0, and a GPIO
edge on the built-in D13 LED. The build and simulator artifacts are committed
under [`evidence/`](evidence/). This is local simulator evidence, not hardware
confirmation.

## Model boundary

The smoke covers the AVR core, GPIO, USART0, ADC, and Timer0 used by `millis()`.
It does not claim USB behavior or the ATmega16U2 bridge, USART1–3, Timer3–5,
INTn/PCINT, or PWM pads on the Timer3–5 pins (the Timer0/1/2 pads are modelled
and tested by `avr_mega2560_pwm`). Those unsupported paths are listed in the chip
profile and [`docs/boards/arduino-mega-2560.md`](../../docs/boards/arduino-mega-2560.md).
