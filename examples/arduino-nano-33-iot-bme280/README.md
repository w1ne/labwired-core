# Arduino Nano 33 IoT + BME280

Run commands from the repository root.

Demand position: Arduino kits rank 3 at 37% in the JetBrains State of Embedded
survey. The ATSAMD21G18A is not ranked separately. It is the MCU of the
Arduino Zero, the MKR boards, the Nano 33 IoT and most "M0" hobby boards, and
on nearly all of them `Serial` is the chip's native USB port. Live stock
evidence is in [`vendor-status.md`](vendor-status.md).

This example runs a stock sketch whose console is native USB. The scripted
USB host enumerates the board and opens its CDC serial port, as a terminal
does. The stock Adafruit BME280 library then reads the sensor over `Wire` and
the sketch prints temperature, pressure and humidity on `Serial`.

## Hardware and wiring

Use an Arduino Nano 33 IoT (ABX00027) and the Adafruit BME280 breakout
(PID 2652). The Nano 33 IoT is a 3.3 V board; the breakout accepts 3.3 V on
VIN.

| Nano 33 IoT | BME280 breakout | Purpose |
| --- | --- | --- |
| 3.3V | VIN | breakout supply |
| GND | GND | common ground |
| A4 (PB08) | SDI / SDA | SERCOM4 PAD0, SDA |
| A5 (PB09) | SCK / SCL | SERCOM4 PAD1, SCL |

The breakout's default address is `0x77`. Its board includes the I2C
pull-ups. The on-board LSM6DS3 IMU (0x6A) and ATECC608A (0x60) share this bus
on the real board; they are not attached in the simulated system.

## Copy-paste Arduino example

Select **Arduino Nano 33 IoT**, install **Adafruit BME280 Library** (2.3.0 was
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
  Serial.print("NANO_33_IOT_BME280 ID=0x");
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
committed build is `tests/fixtures/samd/arduino-nano-33-iot-bme280.elf`;
`firmware/build.sh` rebuilds it and copies it there.

## Build and simulate

```bash
python -m pip install platformio
examples/arduino-nano-33-iot-bme280/firmware/build.sh
cargo run -q -p labwired-cli -- test \
  --script examples/arduino-nano-33-iot-bme280/bme280-smoke.yaml --no-uart-stdout
cargo test -p labwired-core --test samd arduino_nano_33_iot_bme280::
```

Expected serial output with the sensor model at its defaults (25 °C,
1013.25 hPa, 50 %RH):

```text
NANO_33_IOT_BME280 ID=0x60
T=25.00 C
P=1013.25 hPa
RH=50.00 %
```

The first line appears after about 510 ms of simulated time: ArduinoCore-samd
reports the port closed for the first 500 ms after reset. Set
`LABWIRED_TRACE_USB=1` to log each control transfer of the host.

The Rust test runs on pull requests from the committed ELF. It needs no
PlatformIO.

## Evidence

| Run | Result | Record |
| --- | --- | --- |
| `bme280-smoke.yaml` | PASS, 16/16 checks | [`evidence/run/`](evidence/run/) |
| Value control, sensor at 31.5 °C, 990 hPa, 40 %RH ([`controls/warm-room.yaml`](controls/warm-room.yaml)) | PASS, prints `T=31.50 C`, `P=990.00 hPa`, `RH=40.00 %` | [`evidence/controls/warm-room/`](evidence/controls/warm-room/) |
| Control, no sensor on SERCOM4 ([`controls/no-sensor.yaml`](controls/no-sensor.yaml)) | PASS, prints `BME280 not found at 0x77` and no reading | [`evidence/controls/no-sensor/`](evidence/controls/no-sensor/) |
| Control, no cable in the USB port ([`controls/no-host.yaml`](controls/no-host.yaml)) | FAIL as required, no serial output | [`evidence/controls/no-host/`](evidence/controls/no-host/) |
| Control, USB block as the register bank it was before this change ([`controls/usb-register-bank.yaml`](controls/usb-register-bank.yaml)) | FAIL as required, no serial output | [`evidence/controls/usb-register-bank/`](evidence/controls/usb-register-bank/) |

This is repository smoke evidence. It is not a `labwired_verify`
model-verification verdict. No physical board test was run.

## Model boundary

Modelled and exercised: the USB device controller (`samd21_usb`): the RAM
endpoint descriptor table, SETUP / IN / OUT on a control endpoint 0, bulk and
interrupt endpoints, multi-packet IN (the 75-byte configuration descriptor),
bus reset, start of frame every millisecond and the single USB interrupt;
SERCOM4 as the I2C controller for `Wire`; SysTick for `delay()` and
`millis()`; PA17 for `LED_BUILTIN`; the BME280 register model with the shipped
calibration coefficients.

The host is scripted. It resets the bus, reads the descriptors, sets address 5
and configuration 1, sends SET_LINE_CODING (115200 8N1) and
SET_CONTROL_LINE_STATE (DTR, RTS), then reads the bulk IN endpoint into the
console. It is not a USB stack and it sends no data to the board: `Serial.read()`
from the host side is not available in `labwired test`.

Refused with an error, not ignored: USB host mode, high speed and the test
modes, remote wakeup, GNAK, NREPLY, LPM, isochronous and dual-bank endpoints.

Not modelled: USB suspend and resume, packet-level timing, CRC and bit-stuff
errors, the GCLK_USB generic clock (PM.APBBMASK and the PA24 / PA25 pin mux
are checked), I2C pad levels,
pull-up voltage, and BME280 conversion time. The NINA-W102 Wi-Fi module, the
ATECC608A and the LSM6DS3 IMU of the board are not attached.
