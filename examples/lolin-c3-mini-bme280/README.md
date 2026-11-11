# WEMOS LOLIN C3 Mini V2.1.0 + BME280

ESP32 is demand rank **#1** in the JetBrains State of Embedded 2025 survey
(38% of reported MCU use). This adds a smoke for the buyable LOLIN C3 Mini
V2.1.0. It is incremental coverage on the existing ESP32-C3 model, not a new
core or vendor.

The firmware reads BME280 ID 0x60 and raw temperature 0x082DC0 over I2C0 on
GPIO4/GPIO5. It toggles exposed GPIO6 only after both reads match. The
descriptor deliberately omits the onboard WS2812B on GPIO7 because the board's
RGB LED is not modeled here.

## Wiring

Connect a BME280 breakout to 3V3, GND, GPIO4 (SDA), and GPIO5 (SCL). On the
V2.1.0 schematic, GPIO4 is J2 pin 6, GPIO5 is J2 pin 7, and ground is J5 pin 7.
GPIO6 is J5 pin 6; connect it to an external LED through a current-limiting
resistor if you want to see the proof signal.

## Copy-paste Arduino sketch

This sketch uses the sensor reads as the gate for GPIO6. The smoke checks the
resulting GPIO edges, so missing or mismatched sensor data cannot pass.

    #include <Arduino.h>
    #include <Wire.h>

    constexpr uint8_t kBme280 = 0x77;
    constexpr int kSda = 4;
    constexpr int kScl = 5;
    constexpr int kProofPin = 6;

    bool readRegisters(uint8_t reg, uint8_t *data, size_t length) {
      Wire.beginTransmission(kBme280);
      if (Wire.write(reg) != 1 || Wire.endTransmission(false) != 0) return false;
      if (Wire.requestFrom(static_cast<uint16_t>(kBme280), length, true) != length)
        return false;
      for (size_t i = 0; i < length; ++i) {
        if (!Wire.available()) return false;
        data[i] = static_cast<uint8_t>(Wire.read());
      }
      return true;
    }

    bool sensorReadPassed = false;

    void setup() {
      pinMode(kProofPin, OUTPUT);
      digitalWrite(kProofPin, LOW);
      Wire.begin(kSda, kScl, 100000);

      uint8_t id = 0;
      uint8_t temp[3] = {};
      if (!readRegisters(0xD0, &id, 1) || id != 0x60 ||
          !readRegisters(0xFA, temp, sizeof(temp))) return;
      uint32_t raw = (uint32_t(temp[0]) << 12) | (uint32_t(temp[1]) << 4) |
                     (temp[2] >> 4);
      sensorReadPassed = raw == 0x082DC0;
    }

    void loop() {
      if (!sensorReadPassed) return;
      digitalWrite(kProofPin, !digitalRead(kProofPin));
      delay(25);
    }

See firmware/src/main.cpp and VALIDATION.md for the exact committed firmware and
reproduce command. The committed smoke exercises C3 mask-ROM boot, I2C0, the
BME280 model, and GPIO6. Wi-Fi, Bluetooth LE, USB CDC, and the WS2812B are
outside this profile.
