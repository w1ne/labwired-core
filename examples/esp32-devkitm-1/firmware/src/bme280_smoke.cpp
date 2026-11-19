// ESP32-DevKitM-1 + BME280 on the standard GPIO21/22 I2C pin pair.
#include <Arduino.h>
#include <Wire.h>

namespace {
constexpr uint8_t kSdaPin = 21;
constexpr uint8_t kSclPin = 22;
constexpr uint8_t kBme280Address = 0x77;
constexpr uint8_t kChipIdRegister = 0xD0;
constexpr uint8_t kBme280ChipId = 0x60;
constexpr uint8_t kIndicatorPin = 23;

constexpr uintptr_t kGpioBase = 0x3ff44000;
constexpr uintptr_t kI2c0Base = 0x3ff53000;
constexpr uint32_t kIndicatorMask = 1u << kIndicatorPin;
constexpr uint32_t kI2cStart = 1u << 5;

volatile uint32_t &reg(uintptr_t address) {
  return *reinterpret_cast<volatile uint32_t *>(address);
}

constexpr uint32_t command(uint8_t opcode, uint8_t byteCount) {
  return (static_cast<uint32_t>(opcode) << 11) | byteCount;
}
} // namespace

// Local fast-boot fixture entry. Run one real classic-ESP32 I2C0 command list
// through GPIO21/22, then raise the external indicator only on BME280 CHIP_ID.
extern "C" __attribute__((used, noinline)) void labwired_entry() {
  reg(kGpioBase + 0x24) = kIndicatorMask; // GPIO_ENABLE_W1TS
  reg(kGpioBase + 0x0c) = kIndicatorMask; // GPIO_OUT_W1TC: start low

  // Classic ESP32 I2C0 GPIO-matrix signals are SDA=30 and SCL=29.
  reg(kGpioBase + 0x530 + 4 * kSdaPin) = 30;
  reg(kGpioBase + 0x530 + 4 * kSclPin) = 29;
  reg(kGpioBase + 0x130 + 4 * 30) = (1u << 7) | kSdaPin;
  reg(kGpioBase + 0x130 + 4 * 29) = (1u << 7) | kSclPin;

  reg(kI2c0Base + 0x18) = 0x3000; // reset RX and TX FIFOs
  reg(kI2c0Base + 0x18) = 0;
  reg(kI2c0Base + 0x00) = 5; // SCL low period
  reg(kI2c0Base + 0x38) = 5; // SCL high period
  reg(kI2c0Base + 0x1c) = (kBme280Address << 1);
  reg(kI2c0Base + 0x1c) = kChipIdRegister;
  reg(kI2c0Base + 0x1c) = (kBme280Address << 1) | 1;

  // Classic ESP32 command opcodes: RSTART=0, WRITE=1, READ=2, STOP=3.
  reg(kI2c0Base + 0x58) = command(0, 0);
  reg(kI2c0Base + 0x5c) = command(1, 2);
  reg(kI2c0Base + 0x60) = command(0, 0);
  reg(kI2c0Base + 0x64) = command(1, 1);
  reg(kI2c0Base + 0x68) = command(2, 1);
  reg(kI2c0Base + 0x6c) = command(3, 0);
  reg(kI2c0Base + 0x04) = 0x03 | kI2cStart;

  if ((reg(kI2c0Base + 0x1c) & 0xff) == kBme280ChipId) {
    reg(kGpioBase + 0x08) = kIndicatorMask; // GPIO_OUT_W1TS
  }

  for (;;) asm volatile("nop");
}

// Copy-paste Arduino smoke for a BME280 breakout at address 0x77.
void setup() {
  Serial.begin(115200);
  pinMode(kIndicatorPin, OUTPUT);
  digitalWrite(kIndicatorPin, LOW);
  Wire.begin(kSdaPin, kSclPin, 100000);

  Wire.beginTransmission(kBme280Address);
  Wire.write(kChipIdRegister);
  if (Wire.endTransmission(false) != 0) {
    Serial.println("DEVKITM1_BME280_I2C_ERROR");
    return;
  }
  if (Wire.requestFrom(kBme280Address, static_cast<uint8_t>(1)) != 1) {
    Serial.println("DEVKITM1_BME280_I2C_ERROR");
    return;
  }

  const uint8_t id = Wire.read();
  if (id == kBme280ChipId) {
    digitalWrite(kIndicatorPin, HIGH);
    Serial.println("DEVKITM1_BME280_OK ID=0x60");
  } else {
    Serial.printf("DEVKITM1_BME280_BAD_ID=0x%02X\n", id);
  }
}

void loop() { delay(100); }
