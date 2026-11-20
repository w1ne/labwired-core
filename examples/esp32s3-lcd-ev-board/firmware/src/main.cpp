// ESP32-S3-LCD-EV-Board v1.5 + SUB2 FT5x06 touch smoke.
//
// setup() is the copy-paste Arduino path. labwired_entry() skips Arduino/IDF
// startup and performs the same I2C reads through the local simulator's S3
// register model so the committed smoke stays fast and deterministic.
#include <Arduino.h>
#include <Wire.h>

namespace {
constexpr uint8_t kSdaPin = 47;
constexpr uint8_t kSclPin = 48;
constexpr uint8_t kTouchAddress = 0x38;
constexpr uint8_t kTouchStatusReg = 0x02;

constexpr uintptr_t kUart0Fifo = 0x60000000;
constexpr uintptr_t kUart0Status = 0x6000001C;
constexpr uintptr_t kGpioBase = 0x60004000;
constexpr uintptr_t kGpioEnable1W1ts = kGpioBase + 0x30;
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

void uartDecimal(uint16_t value) {
  char digits[6];
  uint8_t count = 0;
  do {
    digits[count++] = static_cast<char>('0' + value % 10);
    value /= 10;
  } while (value != 0 && count < sizeof(digits));
  while (count > 0) {
    const char digit[] = {digits[--count], '\0'};
    uartText(digit);
  }
}

void i2cCommand(uint32_t index, uint32_t opcode, uint32_t byteCount,
                uint32_t flags) {
  writeReg(kI2cCommand0 + index * 4,
           (opcode << 11) | (byteCount & 0xFFu) | flags);
}

void setupFastBootI2c() {
  // GPIO47/48 are in GPIO_ENABLE1 (pins 32..48). The I2C0 matrix outputs are
  // 90=SDA and 89=SCL on the ESP32-S3.
  writeReg(kGpioOutSelSda, 90);
  writeReg(kGpioOutSelScl, 89);
  writeReg(kGpioInSelSda, (1u << 7) | kSdaPin);
  writeReg(kGpioInSelScl, (1u << 7) | kSclPin);
  writeReg(kGpioEnable1W1ts, (1u << (kSdaPin - 32)) | (1u << (kSclPin - 32)));

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

bool readTouchRegister(uint8_t reg, uint8_t &value) {
  writeReg(kI2cFifoConf, kFifoReset);
  writeReg(kI2cFifoConf, kFifoRun);
  writeReg(kI2cIntClear, 0xFFFFFFFFu);
  writeReg(kI2cData, uint32_t(kTouchAddress) << 1);
  writeReg(kI2cData, reg);
  writeReg(kI2cData, (uint32_t(kTouchAddress) << 1) | 1u);
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

bool readTouchPoint(uint8_t &count, uint8_t &id, uint16_t &x, uint16_t &y) {
  uint8_t xh, xl, yh, yl;
  if (!readTouchRegister(kTouchStatusReg, count) ||
      !readTouchRegister(0x03, xh) || !readTouchRegister(0x04, xl) ||
      !readTouchRegister(0x05, yh) || !readTouchRegister(0x06, yl)) {
    return false;
  }
  id = yh >> 4;
  x = (uint16_t(xh & 0x0Fu) << 8) | xl;
  y = (uint16_t(yh & 0x0Fu) << 8) | yl;
  return true;
}

bool readTouchPointWithWire(uint8_t &count, uint8_t &id, uint16_t &x,
                            uint16_t &y) {
  uint8_t frame[5] = {};
  Wire.beginTransmission(kTouchAddress);
  Wire.write(kTouchStatusReg);
  if (Wire.endTransmission(false) != 0 || Wire.requestFrom(kTouchAddress, 5) != 5) {
    return false;
  }
  for (uint8_t &byte : frame) byte = Wire.read();
  count = frame[0] & 0x0Fu;
  id = frame[3] >> 4;
  x = (uint16_t(frame[1] & 0x0Fu) << 8) | frame[2];
  y = (uint16_t(frame[3] & 0x0Fu) << 8) | frame[4];
  return true;
}

bool expectedPoint(uint8_t count, uint8_t id, uint16_t x, uint16_t y) {
  return count == 1 && id == 0 && x == 301 && y == 77;
}

void printFastBootResult() {
  uint8_t count = 0, id = 0;
  uint16_t x = 0, y = 0;
  if (!readTouchPoint(count, id, x, y)) {
    uartText("FT5X06_I2C_ERROR\n");
    return;
  }
  if (!expectedPoint(count, id, x, y)) {
    uartText("FT5X06_FRAME_ERROR COUNT=");
  } else {
    uartText("LCD_EV_TOUCH_OK COUNT=");
  }
  uartDecimal(count);
  uartText(" ID=");
  uartDecimal(id);
  uartText(" X=");
  uartDecimal(x);
  uartText(" Y=");
  uartDecimal(y);
  uartText("\n");
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
  uint8_t count = 0, id = 0;
  uint16_t x = 0, y = 0;
  if (readTouchPointWithWire(count, id, x, y) && expectedPoint(count, id, x, y)) {
    Serial.printf("LCD_EV_TOUCH_OK COUNT=%u ID=%u X=%u Y=%u\n", count, id, x, y);
  } else {
    Serial.println("FT5X06_I2C_OR_FRAME_ERROR");
  }
}

void loop() { delay(1000); }
