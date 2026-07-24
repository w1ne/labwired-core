// LabWired Arduino matrix L3 — Wire + INA219 @ 0x40 (system external_devices).
//
// UART: ESP32 Super Mini / C3 builds often map Serial→USB-CDC; LabWired captures
// hardware UART0. Dual-print like marketplace-arduino-c3 so sim always sees text.
//
// I2C pins: ESP boards pass SDA/SCL into Wire.begin(); STM32/nRF/RP use core defaults.

#include <Arduino.h>
#include <Wire.h>

#ifndef INA219_ADDR
#define INA219_ADDR 0x40
#endif

static void logBegin() {
  Serial.begin(115200);
#if defined(ARDUINO_USB_CDC_ON_BOOT) && (ARDUINO_USB_CDC_ON_BOOT)
  Serial.setTxTimeoutMs(0);
#endif
#if defined(ARDUINO_ARCH_ESP32)
  Serial0.begin(115200);
#endif
  delay(1);
}

static void logLine(const char *s) {
  Serial.println(s);
#if defined(ARDUINO_ARCH_ESP32)
  Serial0.println(s);
#endif
}

static void wireBegin() {
#if defined(CONFIG_IDF_TARGET_ESP32C3) || defined(ARDUINO_ESP32C3_DEV) || defined(ARDUINO_ESP32C3_SUPER_MINI)
  // C3 Super Mini / matrix systems: SDA=4 SCL=5
  Wire.begin(4, 5);
#elif defined(CONFIG_IDF_TARGET_ESP32S3) || defined(ARDUINO_ESP32S3_DEV)
  // S3 DevKit: SDA=8 SCL=9 (arduino defaults / systems route)
  Wire.begin(8, 9);
#elif defined(ARDUINO_ARCH_ESP32)
  // Classic ESP32: board defaults (often 21/22) — match systems/esp32.yaml
  Wire.begin(21, 22);
#elif defined(ARDUINO_ARCH_RP2040)
  Wire.begin();
#else
  // STM32 / nRF Arduino cores: default Wire pins for the board profile
  Wire.begin();
#endif
  delay(1);
}

void setup() {
  logBegin();
  logLine("LW_L3_BOOT");
  wireBegin();

  // Probe: write config-reg pointer 0x00 (1 data byte). Empty endTransmission
  // uses i2c_master_probe on modern ESP-IDF HAL and often NACKs under sim;
  // a 1-byte write exercises the same START/ADDR/ACK path as a real sensor
  // access and works on F1/nRF/RP/C3 + classic ESP.
  Wire.beginTransmission(INA219_ADDR);
  Wire.write((uint8_t)0x00);
  uint8_t err = Wire.endTransmission();
  if (err == 0) {
    logLine("LW_L3_OK");
    return;
  }
  char buf[32];
  snprintf(buf, sizeof(buf), "LW_L3_FAIL err=%u", (unsigned)err);
  logLine(buf);
}

void loop() {}
