# Seeed Studio XIAO ESP32-C3

This demand-ranked board adds a buyable ESP32-family system to the simulator.
ESP32 is the #1 MCU family in the referenced JetBrains State of Embedded survey
(38% reported use). See [vendor-status.md](evidence/vendor-status.md) for the
live purchase and pin-map sources.

The narrow LabWired profile loads the ESP32-C3 GPIO model and maps the XIAO's
D0-D10 header pins. The smoke toggles D10 / GPIO10 and asserts at least eight
real GPIO transitions. The XIAO ESP32-C3 has no onboard user LED, so connect an
external LED and a 150 Ω series resistor between D10 and GND to observe it.

## Copy-paste Arduino example

This physical-board example follows Seeed's Getting Started guide. Connect the
external LED and resistor to D10 and GND as described above:

```cpp
const int ledPin = D10;

void setup() {
  pinMode(ledPin, OUTPUT);
}

void loop() {
  digitalWrite(ledPin, HIGH);
  delay(1000);
  digitalWrite(ledPin, LOW);
  delay(1000);
}
```

## Build and run the simulator smoke

From the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id seeed-xiao-esp32c3 \
  --crate firmware-xiao-esp32c3-smoke \
  --target riscv32imc-unknown-none-elf \
  --script examples/seeed-xiao-esp32c3/onboarding-smoke.yaml \
  --system configs/systems/seeed-xiao-esp32c3.yaml \
  --out-dir examples/seeed-xiao-esp32c3/evidence
```

The smoke directly loads a bare-metal ELF. It does not claim boot-ROM,
Arduino-framework, wireless, USB, UART, I2C, SPI, ADC, or timer behavior. This
is simulator evidence, not physical-board testing; see
[`VALIDATION.md`](VALIDATION.md) and the committed `evidence/` logs.

For an I2C sensor design on the same board, see the
[XIAO ESP32-C3 + BME280 example](../seeed-xiao-esp32c3-bme280/README.md).
