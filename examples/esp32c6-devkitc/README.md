# ESP32-C6-DevKitC-1 Onboarding Example

Run all commands from `core/`.

**Demand rank:** ESP32 family is rank 1 in the measured MCU list (38%). This
adds a real C6 I²C sensor path to the existing DevKitC-1 UART board profile.
Live vendor and stock evidence is in [`vendor-status.md`](vendor-status.md).

## Copy-paste Arduino example: BME280 over I²C0

Wire an Adafruit BME280 STEMMA QT breakout to the DevKitC-1:

| ESP32-C6-DevKitC-1 | BME280 breakout | Purpose |
| --- | --- | --- |
| 3V3 | VIN | Sensor supply |
| GND | GND | Common ground |
| GPIO6 | SDA | I²C0 SDA |
| GPIO7 | SCL | I²C0 SCL |

The breakout's SDO default selects address `0x77` and its board has I²C
pull-ups. GPIO6/7 are on header J1. They also have JTAG/LP-I²C alternate
functions, so the sketch explicitly assigns them to the Arduino I²C bus.

```cpp
#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t BME280_ADDRESS = 0x77;
constexpr uint8_t CHIP_ID_REGISTER = 0xD0;

void setup() {
  Serial.begin(115200);
  Wire.begin(6, 7); // GPIO6 SDA, GPIO7 SCL
  Wire.setClock(100000);

  Wire.beginTransmission(BME280_ADDRESS);
  Wire.write(CHIP_ID_REGISTER);
  if (Wire.endTransmission(false) != 0 ||
      Wire.requestFrom(BME280_ADDRESS, static_cast<uint8_t>(1)) != 1) {
    Serial.println("BME280 not found at 0x77");
    return;
  }

  if (Wire.read() == 0x60) {
    Serial.println("ESP32_C6_BME280_OK ID=0x60 I2C0=GPIO6/GPIO7");
  } else {
    Serial.println("Unexpected BME280 chip ID");
  }
}

void loop() { delay(100); }
```

## Build and run the committed smoke

```bash
scripts/onboarding_smoke.sh \
  --target-id esp32c6-devkitc-bme280 \
  --script examples/esp32c6-devkitc/bme280-smoke.yaml \
  --system configs/systems/esp32c6-devkitc-bme280.yaml \
  --out-dir examples/esp32c6-devkitc/evidence/bme280 \
  --firmware-build-script examples/esp32c6-devkitc/build-bme280-firmware.sh
```

The local smoke checks that the C6 I²C0 command engine completed without
NACK, that it returned the BME280 datasheet chip ID (`0x60`), and that the C6
GPIO matrix selectors route GPIO6/GPIO7 to I²C0. Its fast-boot fixture enters
the same compiled Arduino ELF at `labwired_entry` to avoid depending on the
full Arduino startup path. This is a deterministic simulator smoke, not a
physical-board result.

## Purpose

Deterministic L1 bring-up for the **ESP32-C6** HP core (RV32IMAC @ 160 MHz) on
the **ESP32-C6-DevKitC-1** using the minimal supported subset:

1. `pcr` — UART0 APB + function clock gates (the C6's replacement for the C3's
   SYSTEM/APB_CTRL blocks)
2. `io_mux` — GPIO16 routed to U0TXD
3. `uart0` — Espressif UART IP clocked at the C6's rate, `OK\n` to the console
4. `i2c0` — command-list transfer, GPIO matrix route, and BME280 `CHIP_ID`

The firmware prints `OK\n` on the DevKitC-1's USB-to-UART console (U0TXD
GPIO16 / U0RXD GPIO17).

**SIM-DERIVED.** Not silicon-verified. The LP core, ROM boot path, interrupt
matrix, every radio and every other peripheral are out of scope at this level —
see [`docs/boards/esp32c6-devkitc.md`](../../docs/boards/esp32c6-devkitc.md).

## Quick Run

```bash
cargo build -p firmware-esp32c6-demo --release --target riscv32imc-unknown-none-elf
cargo run -q -p labwired-cli -- test \
  --script examples/esp32c6-devkitc/uart-smoke.yaml \
  --output-dir out/esp32c6-devkitc/uart-smoke \
  --no-uart-stdout
```

Expected result:

1. UART smoke contains `OK`
2. BME280 smoke observes `CHIP_ID=0x60` without I²C NACK

## Files

1. `system.yaml`: base board mapping for simulation runs (console and board I/O).
2. `uart-smoke.yaml`: deterministic UART smoke assertion.
3. `bme280-smoke.yaml`: deterministic sensor-path assertion.
4. `vendor-status.md`: dated live availability and pinout references.
5. `REQUIRED_DOCS.md`: source-grounding references (TRM, datasheet, DevKitC-1 guide, esp-idf headers, espressif/svd).
6. `EXTERNAL_COMPONENTS.md`: external component declaration.
7. `VALIDATION.md`: reproducible validation/audit commands.

The C6 board's onboard RGB LED is a WS2812 on GPIO8. This profile does not
claim plain-GPIO LED behavior, Wi-Fi, Bluetooth, 802.15.4, USB, LP-core, or
radio simulation. The BME280 smoke checks only the chip-ID register; it does
not assert environmental conversion data. The chip descriptor's IO_MUX is
register-backed storage, so its GPIO function selection is recorded but not
electrically enforced. Other C6 windows including SPI0/1, I2S0, TWAI1,
security accelerators, and LP I/O/clock blocks remain stubs or register-only
descriptors. The system adds the BME280 on modeled I²C0 only.
