# Raspberry Pi Pico RP2040 PWM servo

**Demand rank:** RP2040 ranks #5 in the measured MCU-use survey (12%). This
extends the existing Raspberry Pi Pico profile with a common PWM actuator
fixture; it is peripheral coverage on an already modeled board, not a new MCU
or board demand pick.

The smoke configures PWM slice 0 channel A on GP16 (Pico header pin 21) for a
50 Hz frame and a 1.5 ms high pulse. It checks the PWM divider, compare and
wrap registers, captures multiple GP16 transitions, and requires the UART
marker. An SG90-compatible servo is an external fixture, not an onboard Pico
part. The Pico's GP25 LED is not exercised by this smoke.

## Wiring for a physical Pico SDK project

Use a separate regulated 5 V servo supply. Join its ground to Pico GND, then
connect the servo signal wire to GP16 (physical pin 21). Do not power the servo
from the Pico's 3V3 pin.

## Copy-paste Pico SDK example

With the Pico C/C++ SDK and the standard 125 MHz `clk_sys` configuration:

```c
#include "pico/stdlib.h"
#include "hardware/pwm.h"

int main(void) {
    const uint gpio = 16;
    gpio_set_function(gpio, GPIO_FUNC_PWM);

    const uint slice = pwm_gpio_to_slice_num(gpio); // PWM slice 0
    const uint channel = pwm_gpio_to_channel(gpio); // channel A
    pwm_config config = pwm_get_default_config();
    pwm_config_set_clkdiv(&config, 125.0f);         // 125 MHz / 125 = 1 MHz
    pwm_config_set_wrap(&config, 19999);            // 20,000 ticks = 50 Hz
    pwm_init(slice, &config, false);
    pwm_set_chan_level(slice, channel, 1500);       // 1.5 ms high pulse
    pwm_set_enabled(slice, true);

    while (true) {
        tight_loop_contents();
    }
}
```

## Run the local onboarding smoke

From the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id pico-pwm-servo \
  --crate firmware-pico-pwm-servo \
  --target thumbv6m-none-eabi \
  --script examples/pico-pwm-servo/onboarding-smoke.yaml \
  --system configs/systems/rp2040-pico-pwm-servo.yaml \
  --firmware-build-dir crates/firmware-pico-pwm-servo \
  --out-dir examples/pico-pwm-servo/evidence
```

Committed build and simulator output are stored under [`evidence/`](evidence/).
The separate hosted behavioral-oracle result is documented in
[`evidence/hosted-verification.md`](evidence/hosted-verification.md).

## Scope

The profile uses the existing RP2040 PWM slice model, IO_BANK0 mux and GPIO pad
route, UART0 capture, and a connected servo fixture. The local smoke checks the
PWM registers and GP16 waveform; the hosted verifier checks its UART marker and
four GP16 edges, but neither asserts decoded servo angle. It does not cover the
Pico boot-ROM/USB UF2 path, USB, Wi-Fi (not present on this Pico variant), or
physical servo movement and power behavior. The RP2040 descriptor's TBMAN
window remains an explicit zero-answering stub and is not accessed here. See
[`KNOWN_LIMITATIONS.md`](KNOWN_LIMITATIONS.md) for the detailed claim boundary.
