# ESP32-S3-DevKitC-1 MCPWM servo system

**Demand rank:** ESP32 family is rank 1 in the standing survey (38% MCU use;
42% of kits). This round adds a board-plus-servo smoke to the existing S3
model. It is system and peripheral depth, not a new MCU or vendor.

The system is an Espressif ESP32-S3-DevKitC-1-N16R8 with its onboard WS2812
LED and an SG90-class servo on the J1 GPIO4 header. The bare-metal smoke boots
on the S3, sends a frame that the onboard LED model decodes as lit, configures
MCPWM0 for a 50 Hz servo signal, and captures the resulting GPIO4 waveform.
The smoke checks the MCPWM compare register and routed pad transitions; it
does not assert servo-angle telemetry.

## Pin map and wiring

| Board connection | MCU signal | Pin | Scope |
| --- | --- | --- | --- |
| External servo signal | GPIO4 / MCPWM0 PWM0A | J1 pin 4 | 50 Hz PWM; 1.5 ms pulse |
| Onboard RGB LED DIN, v1.0 | RMT TX channel 0 | GPIO48 | RMT frame decoded as a lit pixel by the attached NeoPixel model |
| USB-to-UART bridge TX | UART0 TX | GPIO43 / J3 pin 2 | UART smoke marker |

Power the servo from a separate 5 V supply and connect its ground to board
ground. Connect only its signal lead to J1 pin 4. Espressif documents GPIO4 at
J1 pin 4; its guide says the initial and v1.1 revisions are both still on the
market. The RGB LED pin differs by revision: GPIO48 on v1.0 and GPIO38 on v1.1.
The smoke profile targets v1.0 because that exact LED path is included.

## Arduino example

With Arduino-ESP32 3.x selected for **ESP32-S3 Dev Module**, connect an SG90
signal lead to J1 pin 4 / GPIO4. Give the servo a separate 5 V supply and
connect grounds. The tested profile uses MCPWM; this compact Arduino sketch
shows the commonly used LEDC alternative for the same servo wiring:

```cpp
#include <Arduino.h>

constexpr uint8_t SERVO_PIN = 4;
constexpr uint32_t PWM_HZ = 50;
constexpr uint8_t PWM_BITS = 14;

void setup() {
  if (!ledcSetClockSource(LEDC_REF_CLK) ||
      !ledcAttach(SERVO_PIN, PWM_HZ, PWM_BITS)) {
    while (true) {}
  }

  // 1.45 ms high in a 20 ms frame, near 90 degrees on an SG90.
  const uint32_t duty = (1450u * (1u << PWM_BITS) + 10000u) / 20000u;
  ledcWrite(SERVO_PIN, duty);
}

void loop() {}
```

For a standard hobby servo, use pulses around 0.5–2.4 ms for the modeled
0–180° range. Check the servo's own pulse limits before sweeping it.

The register-level MCPWM setup used for the hosted GPIO oracle is available in
[`firmware/mcpwm_hosted_verify.ino`](firmware/mcpwm_hosted_verify.ino). The
bare-metal build used by the local board smoke is
[`firmware/mcpwm_servo_smoke.c`](firmware/mcpwm_servo_smoke.c).

## Reproduce the local smoke

From the repository root:

```sh
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32-s3-devkitc-1-n16r8-v10-servo \
  --script examples/esp32-s3-devkitc-servo/onboarding-smoke.yaml \
  --system configs/systems/esp32-s3-devkitc-1-n16r8-v10-servo.yaml \
  --firmware-build-script examples/esp32-s3-devkitc-servo/ci/build-firmware.sh \
  --out-dir examples/esp32-s3-devkitc-servo/evidence
```

The smoke gates on the UART marker, a lit decoded onboard pixel, the MCPWM0
compare register (`0x6001E040 = 1500`), at least two GPIO4 transitions, and
the expected step-limit stop. See the committed
[`evidence/`](evidence/) directory and the [live availability receipt](evidence/vendor-status.md).

## Scope and limits

- The profile uses fast boot and covers UART0, GPIO4, MCPWM0 PWM0A, GPIO48,
  RMT channel 0, the onboard NeoPixel, and the servo command model.
- The MCPWM timer and generator model produces a 50 Hz waveform at GPIO4.
  The servo model can consume MCPWM duty commits; it does not model servo
  current draw, torque, gear backlash, or external supply behavior.
- RMT transmission completes immediately in the model while the attached
  NeoPixel decodes the transmitted frame. Pixel color is not asserted.
- Wi-Fi, Bluetooth LE, native USB, PSRAM behavior, ROM startup, and other GPIO
  pins are outside this focused profile. The result is local simulation
  evidence, not a physical-board capture.
