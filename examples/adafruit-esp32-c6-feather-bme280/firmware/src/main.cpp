// Adafruit ESP32-C6 Feather + external BME280 on STEMMA QT.
// GPIO20 powers the STEMMA QT rail; GPIO15 is the active-high red LED.

#include <Arduino.h>
#include <Wire.h>

namespace {
constexpr uint8_t kBme280Address = 0x77;
constexpr int kStemmaPowerPin = 20;
constexpr int kSdaPin = 19;
constexpr int kSclPin = 18;
constexpr int kStatusLedPin = 15;
bool bme280ReadPassed = false;

bool readRegisters(uint8_t firstRegister, uint8_t *destination,
                   size_t length) {
  Wire.beginTransmission(kBme280Address);
  if (Wire.write(firstRegister) != 1 || Wire.endTransmission(false) != 0) {
    return false;
  }
  if (Wire.requestFrom(static_cast<uint16_t>(kBme280Address), length, true) !=
      length) {
    return false;
  }
  for (size_t i = 0; i < length; ++i) {
    if (Wire.available() <= 0) return false;
    destination[i] = static_cast<uint8_t>(Wire.read());
  }
  return true;
}
} // namespace

void setup() {
  Serial.begin(115200); // C6 USB Serial/JTAG through the Feather USB-C port.

  pinMode(kStatusLedPin, OUTPUT);
  digitalWrite(kStatusLedPin, LOW);

  // Adafruit's guide requires IO20 high before using the STEMMA QT connector.
  pinMode(kStemmaPowerPin, OUTPUT);
  digitalWrite(kStemmaPowerPin, HIGH);
  Wire.begin(kSdaPin, kSclPin, 100000);

  uint8_t chipId = 0;
  uint8_t rawTemperature[3] = {};
  if (!readRegisters(0xD0, &chipId, 1) || chipId != 0x60 ||
      !readRegisters(0xFA, rawTemperature, sizeof(rawTemperature))) {
    Serial.printf("ADAFRUIT_ESP32C6_FEATHER_BME280_FAIL ID=0x%02X\n", chipId);
    return;
  }

  const uint32_t raw = (static_cast<uint32_t>(rawTemperature[0]) << 12) |
                       (static_cast<uint32_t>(rawTemperature[1]) << 4) |
                       (rawTemperature[2] >> 4);
  bme280ReadPassed = raw == 0x082DC0;
  if (bme280ReadPassed) {
    digitalWrite(kStatusLedPin, HIGH);
    Serial.printf(
        "ADAFRUIT_ESP32C6_FEATHER_BME280_OK ID=0x%02X TEMP_RAW=0x%06lX "
        "I2C0=GPIO19/18 LED=GPIO15 STEMMA_POWER=GPIO20\n",
        chipId, static_cast<unsigned long>(raw));
  } else {
    Serial.printf("ADAFRUIT_ESP32C6_FEATHER_BME280_FAIL TEMP_RAW=0x%06lX\n",
                  static_cast<unsigned long>(raw));
  }
}

void loop() {
  if (!bme280ReadPassed) {
    digitalWrite(kStatusLedPin, LOW);
    delay(100);
    return;
  }
  digitalWrite(kStatusLedPin, !digitalRead(kStatusLedPin));
  delay(5);
}
