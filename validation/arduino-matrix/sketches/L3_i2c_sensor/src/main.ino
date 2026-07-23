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
#if defined(ARDUINO_ARCH_ESP32)
  // C3 Super Mini / marketplace: SDA=4 SCL=5 (match systems/*.yaml route defaults)
  Wire.begin(4, 5);
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

  // Probe only (IsDeviceReady / ACK). Full reg read paths vary by HAL Wire
  // implementation; ACK proves kit attach + master START/ADDR on this bus.
  Wire.beginTransmission(INA219_ADDR);
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
