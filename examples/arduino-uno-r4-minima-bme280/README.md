# Arduino UNO R4 Minima + BME280

## Demand position

Arduino boards appeared in **37% of respondents' main-project kits** (kit rank 3 in the demand brief). The RA4M1 is not separately ranked in the MCU-use table. This adds a sensor path to the already modeled UNO R4 Minima; it is board coverage, not new silicon.

The official Arduino store lists the UNO R4 Minima (ABX00080), and Renesas lists an RA4M1 orderable as Active. See the [live vendor-status receipt](vendor-status.md).

## Wiring

Connect an Adafruit BME280 STEMMA QT breakout:

| BME280 breakout | UNO R4 Minima |
| --- | --- |
| VIN | 3V3 |
| GND | GND |
| SDA | A4 / P101 / SDA1 |
| SCL | A5 / P100 / SCL1 |

The firmware reads address `0x77`, register `0xD0`, and expects the BME280 chip ID `0x60`. The breakout's SDO selection sets the address to `0x77`.

## Copy-paste Arduino sketch

The UNO R4 Minima's `Serial1` uses SCI2 on D0/D1. This keeps the example on the modeled external UART path; USB-C `Serial` uses USBFS and is not in this profile.

```cpp
#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t BME280_ADDRESS = 0x77;

void setup() {
  Serial1.begin(115200);  // SCI2 on D0/D1
  Wire.begin();           // IIC1 on A4/A5
  Wire.setClock(100000);

  Wire.beginTransmission(BME280_ADDRESS);
  Wire.write(0xD0);       // BME280 CHIP_ID register
  if (Wire.endTransmission(false) != 0 ||
      Wire.requestFrom(static_cast<int>(BME280_ADDRESS), 1) != 1) {
    Serial1.println("BME280_I2C_ERROR");
    return;
  }

  const uint8_t id = static_cast<uint8_t>(Wire.read());
  Serial1.print("BME280_ID=0x");
  Serial1.println(id, HEX);
}

void loop() {}
```

## Reproduce the committed smoke

```sh
RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id arduino-uno-r4-minima-bme280 \
  --script examples/arduino-uno-r4-minima-bme280/bme280-smoke.yaml \
  --system configs/systems/arduino-uno-r4-minima-bme280.yaml \
  --firmware-build-script examples/arduino-uno-r4-minima-bme280/ci/build-bme280-firmware.sh \
  --out-dir examples/arduino-uno-r4-minima-bme280/evidence
```

The committed smoke builds and boots a bare-metal RA4M1 image through its Cortex-M reset entry, performs an IIC1 transaction, and checks the returned sensor ID. It uses a small register-level driver because ArduinoCore-renesas initializes USBFS during startup; USBFS is still a stub in the engine. The copy-paste sketch shows the corresponding `Wire` + `Serial1` path for a physical board. The sensor model supplies deterministic register values; it does not simulate changing environmental conditions.

The separate [`sci2-regression` evidence](evidence/sci2-regression/) reruns the existing UNO R4 Minima UART smoke after enabling MSTP gating for SCI2.

## Model scope

This profile exposes SCI2 on D0/D1, IIC1 to the BME280 on A4/A5, and the D13 LED. The RA MSTP register model uses reset-stopped bits to gate SCI2 and IIC1; other MSTP-controlled blocks are outside this profile. PRCR write protection is not modeled. It does not claim the Minima's USB-C CDC path, SPI, ADC, CAN, DAC, or other RA4M1 peripherals. `usbfs` remains a zero-answering stub in the chip descriptor and is not accessed here. The local RIIC model checks the controller transaction and the BME280 register response; it does not check the IIC pin mux, bus voltage, pull-ups, or power sequencing.
