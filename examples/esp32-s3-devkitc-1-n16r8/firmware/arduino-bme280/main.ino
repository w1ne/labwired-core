#include <Arduino.h>
#include <Wire.h>

constexpr uint8_t kBme280Address = 0x77;
volatile uint32_t *const kUart0Fifo =
    reinterpret_cast<volatile uint32_t *>(0x60000000u);

void uartByte(char value) { *kUart0Fifo = static_cast<uint8_t>(value); }

void uartText(const char *text) {
  while (*text) uartByte(*text++);
}

bool readRegisters(uint8_t first, uint8_t *bytes, size_t count) {
  Wire.beginTransmission(kBme280Address);
  Wire.write(first);
  if (Wire.endTransmission(false) != 0) return false;
  if (Wire.requestFrom(kBme280Address, static_cast<uint8_t>(count)) != count) {
    return false;
  }
  for (size_t i = 0; i < count; ++i) bytes[i] = Wire.read();
  return true;
}

void printHexByte(uint8_t value) {
  constexpr char kHex[] = "0123456789ABCDEF";
  uartByte(kHex[value >> 4]);
  uartByte(kHex[value & 0x0F]);
}

void printHex20(uint32_t value) {
  printHexByte(static_cast<uint8_t>(value >> 16));
  printHexByte(static_cast<uint8_t>(value >> 8));
  printHexByte(static_cast<uint8_t>(value));
}
void setup() {
  Wire.begin(8, 9, 100000);  // SDA=GPIO8, SCL=GPIO9

  uint8_t id = 0;
  if (!readRegisters(0xD0, &id, 1)) {
    uartText("BME280_I2C_ERROR\n");
    return;
  }
  uartText("BME280_ID=0x");
  printHexByte(id);
  uartText("\n");
  if (id != 0x60) {
    uartText("BME280_ID_ERROR\n");
    return;
  }

  uint8_t temperature[3] = {};
  if (!readRegisters(0xFA, temperature, sizeof(temperature))) {
    uartText("BME280_I2C_ERROR\n");
    return;
  }
  const uint32_t raw = (static_cast<uint32_t>(temperature[0]) << 12) |
                       (static_cast<uint32_t>(temperature[1]) << 4) |
                       (temperature[2] >> 4);
  uartText("BME280_TEMP_RAW=0x");
  printHex20(raw);
  uartText("\n");
}

void loop() {}
