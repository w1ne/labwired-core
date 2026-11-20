# Arduino Nano 33 BLE Rev2 + BME280

Run commands from the repository root.

Demand position: Arduino kits rank 3 at 37% in the JetBrains State of Embedded
survey. The nRF52840 is not ranked separately. It is the MCU of the Nano 33
BLE boards, the nRF52840 DK and most hobby BLE boards. Live stock evidence is
in [`vendor-status.md`](vendor-status.md): the Arduino store is sold out
today; one authorized distributor lists stock.

This example runs a stock ArduinoCore-mbed sketch whose console is native
USB. The scripted USB host enumerates the board and opens its CDC serial
port, as a terminal does. The stock Adafruit BME280 library then reads the
sensor over `Wire` and the sketch prints temperature, pressure and humidity
on `Serial`.

## Hardware and wiring

Use an Arduino Nano 33 BLE Rev2 (ABX00071) and the Adafruit BME280 breakout
(PID 2652). The Nano 33 BLE is a 3.3 V board; the breakout accepts 3.3 V on
VIN.

| Nano 33 BLE | BME280 breakout | Purpose |
| --- | --- | --- |
| 3.3V | VIN | breakout supply |
| GND | GND | common ground |
| A4 (P0.31) | SDI / SDA | Wire SDA |
| A5 (P0.02) | SCK / SCL | Wire SCL |

The breakout's default address is `0x77`. Its board includes the I2C
pull-ups. The on-board BMI270 and BMM150 are on a second bus (`Wire1`) and
are not attached in the simulated system.

## Copy-paste Arduino example

Select **Arduino Nano 33 BLE**, install **Adafruit BME280 Library** (2.3.0 was
used here), and upload:

```cpp
#include <Arduino.h>
#include <Wire.h>
#include <Adafruit_BME280.h>

Adafruit_BME280 bme;

void setup() {
  pinMode(LED_BUILTIN, OUTPUT);
  Serial.begin(115200);
  // Native USB: wait until a terminal opens the port (DTR), or the first
  // lines are lost.
  while (!Serial) {
    delay(1);
  }
  if (!bme.begin(0x77)) {
    Serial.println("BME280 not found at 0x77");
    return;
  }
  Serial.print("NANO_33_BLE_BME280 ID=0x");
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
committed build is `tests/fixtures/nrf52/arduino-nano-33-ble-bme280.elf`
(PlatformIO `nordicnrf52` 11.0.0, ArduinoCore-mbed 4.6.0);
`firmware/build.sh` rebuilds it and copies it there.

## Build and simulate

```bash
python -m pip install platformio
examples/arduino-nano-33-ble-bme280/firmware/build.sh
cargo run -q -p labwired-cli -- test \
  --script examples/arduino-nano-33-ble-bme280/bme280-smoke.yaml --no-uart-stdout
cargo test -p labwired-core --test nrf arduino_nano_33_ble_bme280::
```

Expected serial output with the sensor model at its defaults (25 °C,
1013.25 hPa, 50 %RH):

```text
NANO_33_BLE_BME280 ID=0x60
T=25.00 C
P=1013.25 hPa
RH=50.00 %
```

The Rust test runs on pull requests from the committed ELF. It needs no
PlatformIO.

## Evidence

| Run | Result | Record |
| --- | --- | --- |
| `bme280-smoke.yaml` | PASS, 17/17 checks | [`evidence/run/`](evidence/run/) |
| Value control, sensor at 31.5 °C, 990 hPa, 40 %RH ([`controls/warm-room.yaml`](controls/warm-room.yaml)) | PASS, prints `T=31.50 C`, `P=990.00 hPa`, `RH=40.00 %` | [`evidence/controls/warm-room/`](evidence/controls/warm-room/) |
| Control, no sensor on Wire ([`controls/no-sensor.yaml`](controls/no-sensor.yaml)) | PASS, prints `BME280 not found at 0x77` and no reading | [`evidence/controls/no-sensor/`](evidence/controls/no-sensor/) |
| Control, no cable in the USB port ([`controls/no-host.yaml`](controls/no-host.yaml)) | FAIL as required, no serial output | [`evidence/controls/no-host/`](evidence/controls/no-host/) |

All four runs are from 2026-11-20 on a debug build of the CLI.

This is repository smoke evidence. It is not a `labwired_verify`
model-verification verdict. No physical board test was run.

## Model boundary

Modelled and exercised: the nRF52840 USB device controller
(`nrf52840_usbd`) with the start-up sequence the nrfx driver runs (the
anomaly 171 and 187 workarounds, the VBUS and regulator events of POWER),
control transfers on endpoint 0, EasyDMA bulk and interrupt endpoints, bus
reset and the USBD interrupt; the TWI0 master with byte registers
(`ENABLE = 5`), which Mbed OS uses for `Wire`; RTC1 and the LFCLK for the
Mbed OS tick; P0.13 for `LED_BUILTIN`; the BME280 register model with the
shipped calibration coefficients.

The host is scripted. It resets the bus, reads the descriptors, sets the
address and configuration 1, sends SET_LINE_CODING (115200 8N1) and
SET_CONTROL_LINE_STATE (DTR, RTS), then reads the bulk IN endpoint into the
console. It is not a USB stack and it sends no data to the board:
`Serial.read()` from the host side is not available in `labwired test`.

The board's bootloader is not modelled. The manifest enters the application
at 0x10000 (`memory_overrides.reset_vector_offset`) and starts UARTE0 as the
bootloader leaves it (`bootloader_rx_running`).

Refused with an error, not ignored: the isochronous endpoint, remote wake-up
and `LOWPOWER`.

Not modelled: USB suspend and resume, packet-level
timing, CRC and bit-stuff errors; a data-byte NACK on TWI (attached device
models acknowledge every byte); I2C pad levels, pull-up voltage, and BME280
conversion time. The radio's BLE stack (`ArduinoBLE`), the BMI270 and the
BMM150 of the board are not attached or exercised.
