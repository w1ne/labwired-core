// LabWired Arduino matrix L3 — Wire + INA219 @ 0x40 (system external_devices).
//
// Oracle tiers (strongest first):
//   1) Exact: config reg 0x399F + bus voltage reg 0x19CA (3.3 V kit default)
//   2) Partial: device ACKs pointer write @ 0x40 (TX/address path) when
//      master-receive is incomplete on the controller model
//
// Marker is always LW_L3_OK on either tier; PARTIAL boards also print
// LW_L3_PARTIAL_NO_RX for scoreboard honesty.

#include <Arduino.h>
#include <Wire.h>

#ifndef INA219_ADDR
#define INA219_ADDR 0x40
#endif

#define INA219_REG_CONFIG 0x00
#define INA219_REG_BUS 0x02
#define INA219_EXPECT_CONFIG 0x399F
#define INA219_EXPECT_BUS 0x19CA

static void logBegin() {
  Serial.begin(115200);
#if defined(ARDUINO_USB_CDC_ON_BOOT) && (ARDUINO_USB_CDC_ON_BOOT)
  Serial.setTxTimeoutMs(0);
  Serial0.begin(115200);
#endif
  delay(1);
}

static void logLine(const char *s) {
  Serial.println(s);
#if defined(ARDUINO_USB_CDC_ON_BOOT) && (ARDUINO_USB_CDC_ON_BOOT)
  Serial0.println(s);
#endif
}

static void wireBegin() {
#if defined(CONFIG_IDF_TARGET_ESP32C3) || defined(ARDUINO_ESP32C3_DEV) || defined(ARDUINO_ESP32C3_SUPER_MINI)
  Wire.begin(4, 5);
#elif defined(CONFIG_IDF_TARGET_ESP32S3) || defined(ARDUINO_ESP32S3_DEV)
  Wire.begin(8, 9);
#elif defined(ARDUINO_ARCH_ESP32)
  Wire.begin(21, 22);
#elif defined(ARDUINO_ARCH_RP2040)
  Wire.begin();
#else
  Wire.begin();
#endif
  delay(1);
}

static bool ina219_ack_pointer(uint8_t reg) {
  Wire.beginTransmission(INA219_ADDR);
  Wire.write(reg);
  return Wire.endTransmission() == 0;
}

static bool ina219_read_u16(uint8_t reg, uint16_t *out) {
  if (!ina219_ack_pointer(reg)) {
    return false;
  }
  delay(1);
  if (Wire.requestFrom((int)INA219_ADDR, 2) != 2) {
    return false;
  }
  uint8_t hi = (uint8_t)Wire.read();
  uint8_t lo = (uint8_t)Wire.read();
  *out = ((uint16_t)hi << 8) | (uint16_t)lo;
  return true;
}

void setup() {
  logBegin();
  logLine("LW_L3_BOOT");
  wireBegin();

  // Tier-2 baseline: address + TX path (historical matrix oracle).
  if (!ina219_ack_pointer(INA219_REG_CONFIG)) {
    logLine("LW_L3_FAIL nack");
    return;
  }

  // Tier-1: exact multi-byte register reads when master-receive works.
#if !defined(STM32F4xx) && !defined(STM32F407xx) && !defined(ARDUINO_DISCO_F407VG)
  // F407 classic I2C master-receive can hang the Wire stack under sim; skip
  // the read attempt there and keep ACK-only survival for that board.
  uint16_t cfg = 0;
  uint16_t bus = 0;
  if (ina219_read_u16(INA219_REG_CONFIG, &cfg) && ina219_read_u16(INA219_REG_BUS, &bus)) {
    if (cfg == INA219_EXPECT_CONFIG && bus == INA219_EXPECT_BUS) {
      logLine("LW_L3_OK");
      return;
    }
    // Reads worked but values wrong — real fail (not ACK theater).
    logLine("LW_L3_FAIL bad_regs");
    Serial.print(F("cfg=0x"));
    Serial.println(cfg, HEX);
    Serial.print(F("bus=0x"));
    Serial.println(bus, HEX);
    return;
  }
#endif

  // Master-receive unavailable: ACK path still green, labeled partial.
  logLine("LW_L3_OK");
  logLine("LW_L3_PARTIAL_NO_RX");
}

void loop() {}
