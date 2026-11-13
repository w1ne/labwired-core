// Raspberry Pi Pico (RP2040) + Adafruit BME280 breakout on I2C0.
#include <Arduino.h>
#include <Wire.h>

namespace {
constexpr uint8_t kBme280Address = 0x77;
constexpr uint8_t kChipIdRegister = 0xD0;
constexpr uint8_t kBme280ChipId = 0x60;
constexpr uint32_t kLedMask = 1u << 25;

constexpr uintptr_t kResetsBase = 0x4000C000;
constexpr uintptr_t kResetClear = kResetsBase + 0x3000;
constexpr uintptr_t kResetDone = kResetsBase + 0x08;
constexpr uint32_t kResetI2c0 = 1u << 3;
constexpr uint32_t kResetIoBank0 = 1u << 5;
constexpr uint32_t kResetPadsBank0 = 1u << 8;
constexpr uint32_t kBoardPeripherals = kResetI2c0 | kResetIoBank0 | kResetPadsBank0;

constexpr uintptr_t kIoBank0 = 0x40014000;
constexpr uintptr_t kSio = 0xD0000000;
constexpr uintptr_t kI2c0 = 0x40044000;
constexpr uint32_t kFuncI2c = 3;
constexpr uint32_t kFuncSio = 5;
constexpr uint32_t kI2cRead = 1u << 8;
constexpr uint32_t kI2cStop = 1u << 9;
constexpr uint32_t kI2cRestart = 1u << 10;
constexpr uint32_t kI2cTxAbort = 1u << 6;
constexpr uint32_t kI2cStopDetected = 1u << 9;

volatile uint32_t &reg(uintptr_t address) {
  return *reinterpret_cast<volatile uint32_t *>(address);
}

bool readBme280ChipId(uint8_t *chipId) {
  // Reset release and pin mux used by the Pico's GPIO/I2C paths.
  reg(kResetClear) = kBoardPeripherals;
  for (uint32_t timeout = 0; timeout < 10000; ++timeout) {
    if ((reg(kResetDone) & kBoardPeripherals) == kBoardPeripherals) break;
    if (timeout == 9999) return false;
  }

  // RP2040 I2C0's default Pico route is SDA=GP4, SCL=GP5 (FUNCSEL 3).
  reg(kIoBank0 + 0x04 + 8 * 4) = kFuncI2c;
  reg(kIoBank0 + 0x04 + 8 * 5) = kFuncI2c;

  // The onboard LED is GP25, selected as SIO output. It turns on only after
  // the attached sensor has returned its datasheet chip ID.
  reg(kIoBank0 + 0x04 + 8 * 25) = kFuncSio;
  reg(kSio + 0x18) = kLedMask; // GPIO_OUT_CLR
  reg(kSio + 0x24) = kLedMask; // GPIO_OE_SET

  // Standard-mode master, 7-bit target, repeated starts enabled.
  reg(kI2c0 + 0x6C) = 0;
  reg(kI2c0 + 0x00) = 1u | (1u << 1) | (1u << 5) | (1u << 6) | (1u << 8);
  reg(kI2c0 + 0x14) = 600;
  reg(kI2c0 + 0x18) = 650;
  reg(kI2c0 + 0x04) = kBme280Address;
  reg(kI2c0 + 0x6C) = 1;
  const uint32_t staleInterrupts = reg(kI2c0 + 0x40); // clear stale state
  (void)staleInterrupts;

  // Write CHIP_ID's register address, issue a repeated-start read, then STOP.
  reg(kI2c0 + 0x10) = kChipIdRegister;
  reg(kI2c0 + 0x10) = kI2cRead | kI2cRestart;
  reg(kI2c0 + 0x10) = kI2cRead | kI2cStop;
  for (uint32_t timeout = 0; timeout < 100000; ++timeout) {
    const uint32_t status = reg(kI2c0 + 0x34);
    if ((status & kI2cTxAbort) != 0) {
      const uint32_t abortSource = reg(kI2c0 + 0x54); // clear TX_ABRT
      (void)abortSource;
      return false;
    }
    if ((status & kI2cStopDetected) != 0) {
      const uint32_t stopState = reg(kI2c0 + 0x60); // clear STOP_DET
      (void)stopState;
      *chipId = static_cast<uint8_t>(reg(kI2c0 + 0x10));
      return true;
    }
  }
  return false;
}
} // namespace

// The simulator fixture fast-boots here, after the same register setup and
// actual I2C0 read this board's Arduino sketch performs through Wire below.
extern "C" __attribute__((used, noinline)) void labwired_entry() {
  uint8_t chipId = 0;
  if (readBme280ChipId(&chipId) && chipId == kBme280ChipId) {
    reg(kSio + 0x14) = kLedMask; // GPIO_OUT_SET
  }
  for (;;) asm volatile("nop");
}

// Copy-paste Arduino sketch for the Raspberry Pi Pico and Adafruit BME280.
void setup() {
  Serial.begin(115200);
  pinMode(LED_BUILTIN, OUTPUT);
  digitalWrite(LED_BUILTIN, LOW);

  // The Adafruit BME280 breakout has pull-ups. These internal pull-ups keep
  // the bus idle high if the breakout is temporarily disconnected.
  pinMode(4, INPUT_PULLUP); // GP4 / SDA
  pinMode(5, INPUT_PULLUP); // GP5 / SCL
  Wire.begin();             // Arduino-Pico default I2C0 pins are GP4 / GP5.

  Wire.beginTransmission(kBme280Address);
  Wire.write(kChipIdRegister);
  if (Wire.endTransmission(false) != 0 ||
      Wire.requestFrom(kBme280Address, static_cast<uint8_t>(1)) != 1) {
    Serial.println("BME280 not found at 0x77");
    return;
  }

  const uint8_t chipId = Wire.read();
  if (chipId != kBme280ChipId) {
    Serial.println("Unexpected BME280 chip ID");
    return;
  }

  digitalWrite(LED_BUILTIN, HIGH);
  Serial.println("PICO_BME280_OK ID=0x60 I2C0=GP4/GP5");
}

void loop() { delay(100); }
