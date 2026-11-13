// Seeed Studio XIAO ESP32C3 + Adafruit BME280 breakout on the default Wire pins
// (D4/SDA = GPIO6, D5/SCL = GPIO7). Plain Arduino: the simulator boots the
// full Arduino-ESP32 startup, then this sketch reads the sensor with Wire.
#include <Arduino.h>
#include <Wire.h>

namespace {
constexpr uint8_t kBme280Address = 0x77;
constexpr uint8_t kRegCalibT = 0x88;
constexpr uint8_t kRegChipId = 0xD0;
constexpr uint8_t kRegCtrlMeas = 0xF4;
constexpr uint8_t kRegTempMsb = 0xFA;
constexpr uint8_t kChipId = 0x60;

bool readBytes(uint8_t reg, uint8_t *buf, size_t len) {
  Wire.beginTransmission(kBme280Address);
  Wire.write(reg);
  if (Wire.endTransmission(false) != 0) return false;
  if (Wire.requestFrom(kBme280Address, len) != len) return false;
  for (size_t i = 0; i < len; ++i) buf[i] = Wire.read();
  return true;
}

bool writeByte(uint8_t reg, uint8_t value) {
  Wire.beginTransmission(kBme280Address);
  Wire.write(reg);
  Wire.write(value);
  return Wire.endTransmission() == 0;
}

// Bosch BME280 datasheet, section 4.2.3: temperature in 0.01 degC.
int32_t compensateTemperature(int32_t adcT, uint16_t t1, int16_t t2,
                              int16_t t3) {
  int32_t var1 = ((((adcT >> 3) - (int32_t(t1) << 1))) * int32_t(t2)) >> 11;
  int32_t var2 = (((((adcT >> 4) - int32_t(t1)) *
                    ((adcT >> 4) - int32_t(t1))) >> 12) *
                  int32_t(t3)) >> 14;
  return ((var1 + var2) * 5 + 128) >> 8;
}
} // namespace

void setup() {
  Serial.begin(115200);
  Serial.println("XIAO_C3_BOOT");
  Wire.begin(SDA, SCL, 100000);

  uint8_t id = 0;
  if (!readBytes(kRegChipId, &id, 1)) {
    Serial.println("BME280_I2C_ERROR");
    return;
  }
  Serial.printf("BME280_ID=0x%02X\n", id);
  if (id != kChipId) return;

  uint8_t calib[6];
  uint8_t raw[3];
  // osrs_t = x1, osrs_p = x1, forced mode.
  if (!readBytes(kRegCalibT, calib, sizeof calib) ||
      !writeByte(kRegCtrlMeas, 0x25)) {
    Serial.println("BME280_I2C_ERROR");
    return;
  }
  delay(10);
  if (!readBytes(kRegTempMsb, raw, sizeof raw)) {
    Serial.println("BME280_I2C_ERROR");
    return;
  }
  const uint16_t t1 = uint16_t(calib[1] << 8 | calib[0]);
  const int16_t t2 = int16_t(calib[3] << 8 | calib[2]);
  const int16_t t3 = int16_t(calib[5] << 8 | calib[4]);
  const int32_t adcT =
      int32_t(raw[0]) << 12 | int32_t(raw[1]) << 4 | int32_t(raw[2] >> 4);
  Serial.printf("BME280_DIG_T1=%u\n", t1);
  Serial.printf("BME280_TEMP_RAW=0x%05lX\n", (unsigned long)adcT);
  Serial.printf("BME280_TEMP_CENTI_C=%ld\n",
                (long)compensateTemperature(adcT, t1, t2, t3));
}

void loop() { delay(1000); }
