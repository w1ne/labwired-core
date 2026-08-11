// LabWired Arduino matrix L4 — Nano 33 BLE Sense onboard Wire1 WHO_AM_I suite.
//
// Marker: LW_SENSE_OK when all five IDs match; LW_SENSE_FAIL … otherwise.
// Serial1 only (mbed SerialUSB hangs without a host).

#include <Arduino.h>
#include <Wire.h>

#ifndef PIN_ENABLE_SENSORS_3V3
#define PIN_ENABLE_SENSORS_3V3 33
#endif
#ifndef PIN_ENABLE_I2C_PULLUP
#define PIN_ENABLE_I2C_PULLUP 32
#endif

static void logLine(const char *s) {
  Serial1.println(s);
}

static bool whoAmI(TwoWire &bus, uint8_t addr, uint8_t reg, uint8_t expect) {
  bus.beginTransmission(addr);
  bus.write(reg);
  if (bus.endTransmission(false) != 0) {
    return false;
  }
  if (bus.requestFrom((int)addr, 1) != 1) {
    return false;
  }
  int v = bus.read();
  return v == (int)expect;
}

void setup() {
  pinMode(LED_BUILTIN, OUTPUT);
  digitalWrite(LED_BUILTIN, HIGH);

  Serial1.begin(115200);
  delay(2);
  logLine("LW_SENSE_BOOT");

  pinMode(PIN_ENABLE_SENSORS_3V3, OUTPUT);
  digitalWrite(PIN_ENABLE_SENSORS_3V3, HIGH);
  pinMode(PIN_ENABLE_I2C_PULLUP, OUTPUT);
  digitalWrite(PIN_ENABLE_I2C_PULLUP, HIGH);
  delay(2);

  Wire1.begin();
  delay(1);

  bool ag = whoAmI(Wire1, 0x6B, 0x0F, 0x68);
  bool mag = whoAmI(Wire1, 0x1E, 0x0F, 0x3D);
  bool hts = whoAmI(Wire1, 0x5F, 0x0F, 0xBC);
  bool lps = whoAmI(Wire1, 0x5C, 0x0F, 0xB1);
  bool apds = whoAmI(Wire1, 0x39, 0x92, 0xAB);

  if (ag && mag && hts && lps && apds) {
    logLine("LW_SENSE_OK");
  } else {
    char buf[64];
    snprintf(buf, sizeof(buf), "LW_SENSE_FAIL ag=%u mag=%u hts=%u lps=%u apds=%u",
             (unsigned)ag, (unsigned)mag, (unsigned)hts, (unsigned)lps,
             (unsigned)apds);
    logLine(buf);
  }

  digitalWrite(LED_BUILTIN, LOW);
}

void loop() {
  digitalWrite(LED_BUILTIN, HIGH);
  delay(1);
  digitalWrite(LED_BUILTIN, LOW);
  delay(1);
}
