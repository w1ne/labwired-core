// ESP32-S3-DevKitC-1 v1.1 / N16R8 + BME280 I2C smoke.
//
// setup() is the copy-paste Arduino example. labwired_entry() is the local
// fast-boot harness: it bypasses Arduino startup and runs the same raw I2C
// transaction against the engine's S3 I2C0 model, then writes a UART marker.
#include <Arduino.h>
#include <Wire.h>

namespace {
constexpr uint8_t kSdaPin = 8;
constexpr uint8_t kSclPin = 9;
constexpr uint8_t kBme280Address = 0x77;

constexpr uintptr_t kUart0Fifo = 0x60000000;
constexpr uintptr_t kUart0Status = 0x6000001C;
constexpr uintptr_t kGpioBase = 0x60004000;
constexpr uintptr_t kGpioEnableW1ts = kGpioBase + 0x24;
constexpr uintptr_t kGpioOutSelSda = kGpioBase + 0x554 + kSdaPin * 4;
constexpr uintptr_t kGpioOutSelScl = kGpioBase + 0x554 + kSclPin * 4;
constexpr uintptr_t kGpioInSelSda = kGpioBase + 0x154 + 90 * 4; // I2CEXT0_SDA
constexpr uintptr_t kGpioInSelScl = kGpioBase + 0x154 + 89 * 4; // I2CEXT0_SCL

constexpr uintptr_t kI2cBase = 0x60013000;
constexpr uintptr_t kI2cSclLow = kI2cBase + 0x00;
constexpr uintptr_t kI2cCtr = kI2cBase + 0x04;
constexpr uintptr_t kI2cFifoConf = kI2cBase + 0x18;
constexpr uintptr_t kI2cData = kI2cBase + 0x1C;
constexpr uintptr_t kI2cIntRaw = kI2cBase + 0x20;
constexpr uintptr_t kI2cIntClear = kI2cBase + 0x24;
constexpr uintptr_t kI2cSdaHold = kI2cBase + 0x30;
constexpr uintptr_t kI2cSdaSample = kI2cBase + 0x34;
constexpr uintptr_t kI2cSclHigh = kI2cBase + 0x38;
constexpr uintptr_t kI2cStartHold = kI2cBase + 0x40;
constexpr uintptr_t kI2cRestartSetup = kI2cBase + 0x44;
constexpr uintptr_t kI2cStopHold = kI2cBase + 0x48;
constexpr uintptr_t kI2cStopSetup = kI2cBase + 0x4C;
constexpr uintptr_t kI2cClkConf = kI2cBase + 0x54;
constexpr uintptr_t kI2cCommand0 = kI2cBase + 0x58;

constexpr uint32_t kMasterMode = 1u << 4;
constexpr uint32_t kForceOut = 3u;
constexpr uint32_t kClockEnable = 1u << 8;
constexpr uint32_t kConfUpdate = 1u << 11;
constexpr uint32_t kTransStart = 1u << 5;
constexpr uint32_t kFifoRun = (1u << 14) | (8u << 5) | 1u;
constexpr uint32_t kFifoReset = kFifoRun | (1u << 13) | (1u << 12);
constexpr uint32_t kIntTransComplete = 1u << 7;
constexpr uint32_t kIntNack = 1u << 10;
constexpr uint32_t kOpRestart = 6u;
constexpr uint32_t kOpWrite = 1u;
constexpr uint32_t kOpRead = 3u;
constexpr uint32_t kOpStop = 2u;
constexpr uint32_t kAckCheck = 1u << 8;
constexpr uint32_t kNackLast = 1u << 10;

inline void writeReg(uintptr_t address, uint32_t value) {
  *reinterpret_cast<volatile uint32_t *>(address) = value;
}

inline uint32_t readReg(uintptr_t address) {
  return *reinterpret_cast<volatile uint32_t *>(address);
}

void uartText(const char *text) {
  auto *fifo = reinterpret_cast<volatile uint32_t *>(kUart0Fifo);
  auto *status = reinterpret_cast<volatile uint32_t *>(kUart0Status);
  while (*text != '\0') {
    for (uint32_t i = 0; i < 100000; ++i) {
      if (((*status >> 16) & 0x3FFu) < 128u) break;
    }
    *fifo = static_cast<uint8_t>(*text++);
  }
}

void uartHexByte(uint8_t value) {
  constexpr char kHex[] = "0123456789ABCDEF";
  char text[3] = {kHex[value >> 4], kHex[value & 0x0F], '\0'};
  uartText(text);
}

void uartHex20(uint32_t value) {
  uartHexByte(static_cast<uint8_t>(value >> 16));
  uartHexByte(static_cast<uint8_t>(value >> 8));
  uartHexByte(static_cast<uint8_t>(value));
}

void i2cCommand(uint32_t index, uint32_t opcode, uint32_t byteCount,
                uint32_t flags) {
  writeReg(kI2cCommand0 + index * 4,
           (opcode << 11) | (byteCount & 0xFFu) | flags);
}

void setupFastBootI2c() {
  // S3 GPIO matrix: output signal 90 is I2CEXT0_SDA; 89 is I2CEXT0_SCL.
  writeReg(kGpioOutSelSda, 90);
  writeReg(kGpioOutSelScl, 89);
  writeReg(kGpioInSelSda, (1u << 7) | kSdaPin);
  writeReg(kGpioInSelScl, (1u << 7) | kSclPin);
  writeReg(kGpioEnableW1ts, (1u << kSdaPin) | (1u << kSclPin));

  writeReg(kI2cCtr, kMasterMode | kForceOut | kClockEnable);
  writeReg(kI2cClkConf, 1u << 21); // 40 MHz XTAL source, no divider.
  writeReg(kI2cSclLow, 199);
  writeReg(kI2cSclHigh, 180 | (19u << 9));
  writeReg(kI2cSdaHold, 29);
  writeReg(kI2cSdaSample, 39);
  writeReg(kI2cStartHold, 199);
  writeReg(kI2cRestartSetup, 199);
  writeReg(kI2cStopHold, 199);
  writeReg(kI2cStopSetup, 199);
  writeReg(kI2cCtr,
           kMasterMode | kForceOut | kClockEnable | kConfUpdate);
}

bool readRegister(uint8_t reg, uint8_t &value) {
  writeReg(kI2cFifoConf, kFifoReset);
  writeReg(kI2cFifoConf, kFifoRun);
  writeReg(kI2cIntClear, 0xFFFFFFFFu);
  writeReg(kI2cData, uint32_t(kBme280Address) << 1);
  writeReg(kI2cData, reg);
  writeReg(kI2cData, (uint32_t(kBme280Address) << 1) | 1u);
  i2cCommand(0, kOpRestart, 0, 0);
  i2cCommand(1, kOpWrite, 2, kAckCheck);
  i2cCommand(2, kOpRestart, 0, 0);
  i2cCommand(3, kOpWrite, 1, kAckCheck);
  i2cCommand(4, kOpRead, 1, kNackLast);
  i2cCommand(5, kOpStop, 0, 0);
  writeReg(kI2cCtr,
           kMasterMode | kForceOut | kClockEnable | kTransStart);

  for (uint32_t i = 0; i < 500000; ++i) {
    const uint32_t status = readReg(kI2cIntRaw);
    if (status & kIntNack) return false;
    if (status & kIntTransComplete) {
      value = static_cast<uint8_t>(readReg(kI2cData));
      return true;
    }
  }
  return false;
}

bool readRawTemperature(uint32_t &raw) {
  uint8_t msb, lsb, xlsb;
  if (!readRegister(0xFA, msb) || !readRegister(0xFB, lsb) ||
      !readRegister(0xFC, xlsb)) {
    return false;
  }
  raw = (uint32_t(msb) << 12) | (uint32_t(lsb) << 4) | (xlsb >> 4);
  return true;
}

void printFastBootResult() {
  uint8_t id = 0;
  if (!readRegister(0xD0, id)) {
    uartText("BME280_I2C_ERROR\n");
    return;
  }
  if (id != 0x60) {
    uartText("BME280_ID_ERROR=0x");
    uartHexByte(id);
    uartText("\n");
    return;
  }
  uartText("BME280_ID=0x");
  uartHexByte(id);
  uartText("\n");

  uint32_t raw = 0;
  if (!readRawTemperature(raw)) {
    uartText("BME280_I2C_ERROR\n");
    return;
  }
  uartText("BME280_TEMP_RAW=0x");
  uartHex20(raw);
  uartText("\n");
}

bool readBme280ChipIdWithWire(uint8_t &id) {
  Wire.beginTransmission(kBme280Address);
  Wire.write(0xD0);
  if (Wire.endTransmission(false) != 0 ||
      Wire.requestFrom(kBme280Address, 1) != 1) {
    return false;
  }
  id = Wire.read();
  return true;
}
} // namespace

extern "C" __attribute__((used, noinline)) void labwired_entry() {
  setupFastBootI2c();
  printFastBootResult();
  for (;;) asm volatile("nop");
}

void setup() {
  Serial.begin(115200);
  Wire.begin(kSdaPin, kSclPin, 100000);
  uint8_t id = 0;
  if (readBme280ChipIdWithWire(id) && id == 0x60) {
    Serial.printf("BME280_ID=0x%02X\n", id);
  } else {
    Serial.println("BME280_I2C_ERROR");
  }
}

void loop() { delay(1000); }
