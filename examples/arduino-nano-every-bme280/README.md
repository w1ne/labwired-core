# Arduino Nano Every + BME280

Run commands from the repository root.

Demand position: Arduino kits rank 3 at 37% in the JetBrains State of Embedded
survey. The ATmega4809 is not ranked separately. The survey's #2 MCU is the
ATmega328, which Microchip marks Not Recommended for New Designs; the Nano
Every is the in-production Arduino board in the same form factor. Live stock
evidence is in [`vendor-status.md`](vendor-status.md).

This example adds a real sensor on `Wire`. The stock Adafruit BME280 library
reads the chip ID and the calibration data over TWI0, runs the Bosch
compensation, and prints temperature, pressure and humidity.

## Hardware and wiring

Use an Arduino Nano Every (ABX00028) and the Adafruit BME280 breakout
(PID 2652). The breakout has a regulator and level shifter, so it is safe on
the 5 V board.

| Nano Every | BME280 breakout | Purpose |
| --- | --- | --- |
| 5V | VIN | breakout supply |
| GND | GND | common ground |
| A4 (PA2) | SDI / SDA | TWI0 SDA |
| A5 (PA3) | SCK / SCL | TWI0 SCL |

The breakout's default address is `0x77`. Its board includes the I2C
pull-ups.

## Copy-paste Arduino example

Select **Arduino Nano Every**, install **Adafruit BME280 Library** (2.3.0 was
used here), and upload:

```cpp
#include <Arduino.h>
#include <Wire.h>
#include <Adafruit_BME280.h>

Adafruit_BME280 bme;

void setup() {
  pinMode(LED_BUILTIN, OUTPUT);
  Serial.begin(115200);
  if (!bme.begin(0x77)) {
    Serial.println("BME280 not found at 0x77");
    return;
  }
  Serial.print("NANO_EVERY_BME280 ID=0x");
  Serial.println(bme.sensorID(), HEX);
  Serial.print("T=");
  Serial.print(bme.readTemperature(), 2);
  Serial.println(" C");
  Serial.print("P=");
  Serial.print(bme.readPressure() / 100.0F, 2);
  Serial.println(" hPa");
  Serial.print("RH=");
  Serial.print(bme.readHumidity(), 2);
  Serial.println(" %");
  digitalWrite(LED_BUILTIN, HIGH);
}

void loop() {}
```

The exact sketch is [`firmware/src/main.cpp`](firmware/src/main.cpp). The
committed build is `tests/fixtures/avr/arduino-nano-every-bme280.elf`;
`firmware/build.sh` rebuilds it and copies it there.

## Build and simulate

```bash
python -m pip install platformio
examples/arduino-nano-every-bme280/firmware/build.sh
cargo run -q -p labwired-cli -- test \
  --script examples/arduino-nano-every-bme280/bme280-smoke.yaml --no-uart-stdout
cargo test -p labwired-core --test avr avr_nano_every_bme280::
```

Expected serial output with the sensor model at its defaults (25 °C,
1013.25 hPa, 50 %RH):

```text
NANO_EVERY_BME280 ID=0x60
T=25.00 C
P=1013.25 hPa
RH=50.00 %
```

The Rust test runs on pull requests from the committed ELF. It needs no
PlatformIO.

## Evidence

| Run | Result | Record |
| --- | --- | --- |
| `bme280-smoke.yaml` | PASS, 6/6 checks | [`evidence/run/`](evidence/run/) |
| Value control, sensor at 31.5 °C, 990 hPa, 40 %RH ([`controls/warm-room.yaml`](controls/warm-room.yaml)) | PASS, prints `T=31.50 C`, `P=990.00 hPa`, `RH=40.00 %` | [`evidence/controls/warm-room/`](evidence/controls/warm-room/) |
| Control, no sensor on TWI0 ([`controls/no-sensor.yaml`](controls/no-sensor.yaml)) | FAIL as required, prints `BME280 not found at 0x77` | [`evidence/controls/no-sensor/`](evidence/controls/no-sensor/) |
| Control, descriptor before this change (no TWI0 window) | FAIL as required, `Memory access violation at 0x8a3` (TWI0.MCTRLA) at the first `Wire` access | [`evidence/controls/no-twi0-model/`](evidence/controls/no-twi0-model/) |

This is repository smoke evidence. It is not a `labwired_verify`
model-verification verdict: the hosted verifier catalog has no ATmega4809
target. No physical board test was run.

## Model boundary

Modelled and exercised: the TWI0 host (`megaavr_twi`): START, repeated START,
address, data bytes, STOP, the MBAUD bit time, and the RIF / WIF interrupt on
the TWIM vector that ArduinoCore-megaavr's `Wire` uses; USART3 for `Serial`;
TCB3 for `delay()`; PE2 for `LED_BUILTIN`; the BME280 register model with the
shipped calibration coefficients.

Refused with an error, not ignored: TWI0 client (slave) mode, dual mode, smart
mode, quick command and the inactive-bus time-out.

Not modelled: the TWI0 pads (the sensor answers on any
`PORTMUX.TWISPIROUTEA` route, and SDA / SCL levels are not visible on PA2 /
PA3), pull-up voltage, arbitration loss, bus errors, a device NACK of a data
byte, device clock stretching, and BME280 conversion time. The run assumes
an ideal 5 V rail.
