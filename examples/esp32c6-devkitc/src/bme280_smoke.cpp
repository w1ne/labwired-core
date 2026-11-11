// ESP32-C6-DevKitC-1 + Adafruit BME280 breakout on I2C0.
#include <Arduino.h>
#include <Wire.h>

namespace {
constexpr uint8_t kBme280Address = 0x77;
constexpr uint8_t kChipIdRegister = 0xD0;
constexpr uint8_t kBme280ChipId = 0x60;

constexpr uintptr_t kIoMuxBase = 0x60090000;
constexpr uintptr_t kGpioBase = 0x60091000;
constexpr uintptr_t kI2c0Base = 0x60004000;
constexpr uintptr_t kResultAddress = 0x40800000;

constexpr uint32_t kSdaPin = 6;
constexpr uint32_t kSclPin = 7;
// ESP32-C6 GPIO matrix signal numbers (C6 TRM / esp32c6.svd).
constexpr uint32_t kI2cSclSignal = 45;
constexpr uint32_t kI2cSdaSignal = 46;
constexpr uint32_t kMatrixInputSelect = 1u << 7;
constexpr uint32_t kGpioEnableW1ts = 0x24;
constexpr uint32_t kFuncInSel = 0x154;
constexpr uint32_t kFuncOutSel = 0x554;

constexpr uint32_t kFifoConf = 0x18;
constexpr uint32_t kData = 0x1C;
constexpr uint32_t kIntRaw = 0x20;
constexpr uint32_t kIntClr = 0x24;
constexpr uint32_t kCommand0 = 0x58;
constexpr uint32_t kControl = 0x04;
constexpr uint32_t kTransStart = 1u << 5;
constexpr uint32_t kTransComplete = 1u << 7;
constexpr uint32_t kNack = 1u << 10;
constexpr uint32_t kTxFifoReset = 1u << 13;
constexpr uint32_t kRxFifoReset = 1u << 12;

volatile uint32_t &reg(uintptr_t address) {
  return *reinterpret_cast<volatile uint32_t *>(address);
}

constexpr uint32_t command(uint32_t opcode, uint32_t byteCount) {
  return ((opcode & 0x7u) << 11) | (byteCount & 0xffu);
}
} // namespace

// Fast-boot entry used by the local deterministic CLI smoke. It bypasses the
// Arduino runtime and exercises the C6's actual I2C command engine, GPIO
// matrix route, attached BME280 model, and CHIP_ID register response.
extern "C" __attribute__((used, noinline)) void labwired_entry() {
  auto *result = reinterpret_cast<volatile uint32_t *>(kResultAddress);
  *result = 0xdead0000;

  // Select the GPIO function on pads 6/7 and enable their input paths. The
  // sensor breakout supplies its board-level I2C pull-ups.
  reg(kIoMuxBase + 0x1c) = (1u << 12) | (1u << 9); // GPIO6: MCU_SEL + FUN_IE
  reg(kIoMuxBase + 0x20) = (1u << 12) | (1u << 9); // GPIO7: MCU_SEL + FUN_IE
  reg(kGpioBase + kGpioEnableW1ts) = (1u << kSdaPin) | (1u << kSclPin);

  // C6 matrix numbering is SDA=46 / SCL=45; input SEL is bit 7.
  reg(kGpioBase + kFuncOutSel + 4 * kSdaPin) = kI2cSdaSignal;
  reg(kGpioBase + kFuncOutSel + 4 * kSclPin) = kI2cSclSignal;
  reg(kGpioBase + kFuncInSel + 4 * kI2cSdaSignal) =
      kMatrixInputSelect | kSdaPin;
  reg(kGpioBase + kFuncInSel + 4 * kI2cSclSignal) =
      kMatrixInputSelect | kSclPin;

  // I2C0 uses the C3-compatible command-list opcodes: RSTART=6, WRITE=1,
  // STOP=2, READ=3. Read BME280 register 0xD0 from address 0x77.
  reg(kI2c0Base + kFifoConf) = kTxFifoReset | kRxFifoReset;
  reg(kI2c0Base + kIntClr) = 0xffffffffu;
  reg(kI2c0Base + kCommand0 + 0) = command(6, 0); // RSTART
  reg(kI2c0Base + kCommand0 + 4) = command(1, 2); // address+W, register
  reg(kI2c0Base + kCommand0 + 8) = command(6, 0); // repeated RSTART
  reg(kI2c0Base + kCommand0 + 12) = command(1, 1); // address+R
  reg(kI2c0Base + kCommand0 + 16) = command(3, 1) | (1u << 10); // READ, NACK
  reg(kI2c0Base + kCommand0 + 20) = command(2, 0); // STOP
  reg(kI2c0Base + kData) = kBme280Address << 1;
  reg(kI2c0Base + kData) = kChipIdRegister;
  reg(kI2c0Base + kData) = (kBme280Address << 1) | 1u;
  reg(kI2c0Base + kControl) = kTransStart;

  // TRANS_START auto-clears when the controller accepts the command list, so
  // wait for the asynchronous wire result (complete or NACK), not that bit.
  uint32_t status = 0;
  for (uint32_t timeout = 0; timeout < 100000; ++timeout) {
    status = reg(kI2c0Base + kIntRaw);
    if ((status & (kTransComplete | kNack)) != 0) break;
    if (timeout == 99999) {
      *result = 0xdead0004;
      for (;;) asm volatile("nop");
    }
  }

  if ((status & kNack) != 0) {
    *result = 0xdead0002;
  } else if ((status & kTransComplete) == 0) {
    *result = 0xdead0003;
  } else {
    *result = reg(kI2c0Base + kData) & 0xffu;
  }
  for (;;) asm volatile("nop");
}

// Copy-paste Arduino example for an ESP32-C6-DevKitC-1 and BME280 breakout.
void setup() {
  Serial.begin(115200);
  Wire.begin(kSdaPin, kSclPin);
  Wire.setClock(100000);

  Wire.beginTransmission(kBme280Address);
  Wire.write(kChipIdRegister);
  if (Wire.endTransmission(false) != 0 ||
      Wire.requestFrom(kBme280Address, static_cast<uint8_t>(1)) != 1) {
    Serial.println("BME280 not found at 0x77");
    return;
  }
  if (Wire.read() == kBme280ChipId) {
    Serial.println("ESP32_C6_BME280_OK ID=0x60 I2C0=GPIO6/GPIO7");
  } else {
    Serial.println("Unexpected BME280 chip ID");
  }
}

void loop() { delay(100); }
