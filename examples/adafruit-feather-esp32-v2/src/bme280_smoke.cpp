// Adafruit ESP32 Feather V2 + BME280 on its STEMMA QT connector.
#include <Arduino.h>
#include <Wire.h>

namespace {
constexpr uint8_t kBme280Address = 0x77;
constexpr uint8_t kChipIdRegister = 0xD0;
constexpr uint8_t kBme280ChipId = 0x60;
constexpr uint8_t kStemmaQtPowerPin = 2;
constexpr uint8_t kUserLedPin = 13;

constexpr uintptr_t kGpioBase = 0x3ff44000;
constexpr uintptr_t kI2c0Base = 0x3ff53000;
constexpr uint32_t kLedMask = 1u << kUserLedPin;
constexpr uint32_t kStemmaQtPowerMask = 1u << kStemmaQtPowerPin;
constexpr uint32_t kI2cStart = 1u << 5;

volatile uint32_t &reg(uintptr_t address) {
  return *reinterpret_cast<volatile uint32_t *>(address);
}

constexpr uint32_t command(uint8_t opcode, uint8_t byteCount) {
  return (static_cast<uint32_t>(opcode) << 11) | byteCount;
}
} // namespace

// Local CLI fixture entry: a fast-boot, register-level I2C0 transaction. It
// uses the ESP32 command-list engine and GPIO matrix, then lights GPIO13 only
// if the attached BME280 returned its datasheet CHIP_ID (0x60).
extern "C" __attribute__((used, noinline)) void labwired_entry() {
  reg(kGpioBase + 0x24) = kLedMask | kStemmaQtPowerMask; // GPIO_ENABLE_W1TS
  reg(kGpioBase + 0x08) = kStemmaQtPowerMask;            // GPIO_OUT_W1TS
  reg(kGpioBase + 0x08) = kLedMask;                      // LED high
  reg(kGpioBase + 0x0c) = kLedMask;                      // GPIO_OUT_W1TC

  // Feather V2 STEMMA QT: GPIO22 SDA / GPIO20 SCL. Classic ESP32 I2C0
  // matrix signals are SDA=30 and SCL=29.
  reg(kGpioBase + 0x530 + 4 * 22) = 30; // GPIO_FUNC22_OUT_SEL_CFG
  reg(kGpioBase + 0x530 + 4 * 20) = 29; // GPIO_FUNC20_OUT_SEL_CFG
  reg(kGpioBase + 0x130 + 4 * 30) = (1u << 7) | 22; // I2C SDA input
  reg(kGpioBase + 0x130 + 4 * 29) = (1u << 7) | 20; // I2C SCL input

  reg(kI2c0Base + 0x18) = 0x3000; // reset RX and TX FIFOs
  reg(kI2c0Base + 0x18) = 0;
  reg(kI2c0Base + 0x00) = 5; // SCL low period
  reg(kI2c0Base + 0x38) = 5; // SCL high period
  reg(kI2c0Base + 0x1c) = (kBme280Address << 1);      // address + write
  reg(kI2c0Base + 0x1c) = kChipIdRegister;            // register pointer
  reg(kI2c0Base + 0x1c) = (kBme280Address << 1) | 1;  // address + read

  // Classic ESP32 opcodes: RSTART=0, WRITE=1, READ=2, STOP=3.
  reg(kI2c0Base + 0x58) = command(0, 0);
  reg(kI2c0Base + 0x5c) = command(1, 2);
  reg(kI2c0Base + 0x60) = command(0, 0);
  reg(kI2c0Base + 0x64) = command(1, 1);
  reg(kI2c0Base + 0x68) = command(2, 1);
  reg(kI2c0Base + 0x6c) = command(3, 0);
  reg(kI2c0Base + 0x04) = 0x03 | kI2cStart; // CTR.TRANS_START

  if ((reg(kI2c0Base + 0x1c) & 0xff) == kBme280ChipId) {
    reg(kGpioBase + 0x08) = kLedMask; // CHIP_ID accepted: red LED high
  }

  for (;;) asm volatile("nop");
}

// Copy-paste Arduino sketch for the board's STEMMA QT BME280 breakout.
void setup() {
  pinMode(kStemmaQtPowerPin, OUTPUT);
  digitalWrite(kStemmaQtPowerPin, HIGH); // enables the Feather's QT power rail
  pinMode(kUserLedPin, OUTPUT);
  digitalWrite(kUserLedPin, LOW);
  delay(10);

  Wire.begin(); // Feather V2 defaults: SDA=GPIO22, SCL=GPIO20
  Wire.beginTransmission(kBme280Address);
  Wire.write(kChipIdRegister);
  if (Wire.endTransmission(false) != 0) return;
  if (Wire.requestFrom(kBme280Address, static_cast<uint8_t>(1)) != 1) return;
  if (Wire.read() == kBme280ChipId) digitalWrite(kUserLedPin, HIGH);
}

void loop() { delay(100); }
