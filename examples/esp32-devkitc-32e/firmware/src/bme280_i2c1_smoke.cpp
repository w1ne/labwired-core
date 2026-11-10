#include <Arduino.h>

#include <stdint.h>

namespace {
// Exercise the classic I2C1 command-list peripheral directly. The Arduino
// ESP32 Wire driver currently stalls after a split-command END_DETECT boundary
// in this simulator; this smoke tests the I2C1 controller and physical pin
// route without claiming to validate that higher-level driver path.
constexpr uintptr_t kGpioBase = 0x3FF44000;
constexpr uintptr_t kI2c1Base = 0x3FF67000;
constexpr uintptr_t kGpioEnableW1ts = kGpioBase + 0x24;
constexpr uintptr_t kGpioFuncInSel = kGpioBase + 0x130;
constexpr uintptr_t kGpioFuncOutSel = kGpioBase + 0x530;
constexpr uintptr_t kI2cSclLowPeriod = kI2c1Base + 0x00;
constexpr uintptr_t kI2cCtr = kI2c1Base + 0x04;
constexpr uintptr_t kI2cFifoConf = kI2c1Base + 0x18;
constexpr uintptr_t kI2cData = kI2c1Base + 0x1C;
constexpr uintptr_t kI2cIntRaw = kI2c1Base + 0x20;
constexpr uintptr_t kI2cIntClr = kI2c1Base + 0x24;
constexpr uintptr_t kI2cCmd0 = kI2c1Base + 0x58;
constexpr uintptr_t kI2cSclHighPeriod = kI2c1Base + 0x38;

// GPIO16 = SDA (J3 pin 12); GPIO17 = SCL (J3 pin 11).
constexpr uint8_t kSdaPin = 16;
constexpr uint8_t kSclPin = 17;
constexpr uint8_t kBme280Address = 0x77;
constexpr uint32_t kI2cMasterControl = (1u << 8) | (1u << 4) | 0x3u;
constexpr uint32_t kI2cTransStart = 1u << 5;
constexpr uint32_t kI2cIntTransComplete = 1u << 7;
constexpr uint32_t kI2cIntNack = 1u << 10;
constexpr uint32_t kI2cFifoRxReset = 1u << 12;
constexpr uint32_t kI2cFifoTxReset = 1u << 13;
constexpr uint32_t kI2cAckCheck = 1u << 8;
constexpr uint32_t kI2cNackLast = 1u << 10;

constexpr uint32_t kOpRestart = 0;
constexpr uint32_t kOpWrite = 1;
constexpr uint32_t kOpRead = 2;
constexpr uint32_t kOpStop = 3;

volatile uint32_t &reg(uintptr_t address) {
  return *reinterpret_cast<volatile uint32_t *>(address);
}

void setCommand(uint8_t index, uint32_t opcode, uint8_t byteCount,
                uint32_t flags = 0) {
  reg(kI2cCmd0 + static_cast<uintptr_t>(index) * 4) =
      (opcode << 11) | byteCount | flags;
}

void setupI2c1() {
  reg(kGpioEnableW1ts) = (1u << kSdaPin) | (1u << kSclPin);
  reg(kGpioFuncOutSel + static_cast<uintptr_t>(kSdaPin) * 4) = 96;  // SDA
  reg(kGpioFuncOutSel + static_cast<uintptr_t>(kSclPin) * 4) = 95;  // SCL
  reg(kGpioFuncInSel + 96 * 4) = (1u << 7) | kSdaPin;
  reg(kGpioFuncInSel + 95 * 4) = (1u << 7) | kSclPin;

  // 100 kHz: 400 APB cycles low and high at the classic ESP32's 80 MHz APB.
  reg(kI2cSclLowPeriod) = 400;
  reg(kI2cSclHighPeriod) = 400;
  reg(kI2cCtr) = kI2cMasterControl;
}

bool readRegisters(uint8_t firstRegister, uint8_t *bytes, uint8_t count) {
  reg(kI2cFifoConf) = kI2cFifoRxReset | kI2cFifoTxReset;
  reg(kI2cIntClr) = 0xFFFFFFFF;
  reg(kI2cData) = static_cast<uint32_t>(kBme280Address << 1);
  reg(kI2cData) = firstRegister;
  reg(kI2cData) = static_cast<uint32_t>((kBme280Address << 1) | 1);

  // The classic ESP32 uses RSTART=0, WRITE=1, READ=2, STOP=3.
  setCommand(0, kOpRestart, 0);
  setCommand(1, kOpWrite, 2, kI2cAckCheck);
  setCommand(2, kOpRestart, 0);
  setCommand(3, kOpWrite, 1, kI2cAckCheck);
  setCommand(4, kOpRead, count, kI2cNackLast);
  setCommand(5, kOpStop, 0);
  reg(kI2cCtr) = kI2cMasterControl | kI2cTransStart;

  if ((reg(kI2cIntRaw) & kI2cIntNack) != 0 ||
      (reg(kI2cIntRaw) & kI2cIntTransComplete) == 0) {
    return false;
  }
  for (uint8_t i = 0; i < count; ++i) {
    bytes[i] = static_cast<uint8_t>(reg(kI2cData));
  }
  return true;
}

void readBme280() {
  uint8_t id = 0;
  if (!readRegisters(0xD0, &id, 1)) {
    Serial.println("BME280_I2C1_ERROR");
    return;
  }
  Serial.printf("BME280_I2C1_ID=0x%02X\n", id);
  if (id != 0x60) {
    Serial.println("BME280_I2C1_ID_ERROR");
    return;
  }

  uint8_t temperature[3] = {};
  if (!readRegisters(0xFA, temperature, sizeof(temperature))) {
    Serial.println("BME280_I2C1_ERROR");
    return;
  }
  const uint32_t raw = (static_cast<uint32_t>(temperature[0]) << 12) |
                       (static_cast<uint32_t>(temperature[1]) << 4) |
                       (temperature[2] >> 4);
  Serial.printf("BME280_I2C1_TEMP_RAW=0x%06lX\n",
                static_cast<unsigned long>(raw));
}
}  // namespace

void setup() {
  Serial.begin(115200);
  Serial.println("BME280_I2C1_BOOT");
  setupI2c1();
  readBme280();
}

void loop() {}
