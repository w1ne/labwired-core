// LabWired end-to-end verify for Arduino Nano 33 BLE Sense.
//
// 1. Blink LED_BUILTIN (P0.13)
// 2. Raise sensors 3V3 + I2C pull-ups (required on real hardware and sim policy)
// 3. Wire1 WHO_AM_I probes for soldered suite
// 4. Report markers on Serial1 (hardware UART — sim captures this, not USB CDC)
//
// Expected sim oracles (see test.yaml):
//   - uart_contains: LW_SENSE_OK
//   - gpio0:13 toggled (optional LED watch)

#include <Arduino.h>
#include <Wire.h>

#ifndef PIN_ENABLE_SENSORS_3V3
#define PIN_ENABLE_SENSORS_3V3 33
#endif
#ifndef PIN_ENABLE_I2C_PULLUP
#define PIN_ENABLE_I2C_PULLUP 32
#endif

// Never open SerialUSB — begin() waits for a USB host the sim does not provide.
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

  // Power the sensor rail + internal I2C pull-ups (variant pins).
  pinMode(PIN_ENABLE_SENSORS_3V3, OUTPUT);
  digitalWrite(PIN_ENABLE_SENSORS_3V3, HIGH);
  pinMode(PIN_ENABLE_I2C_PULLUP, OUTPUT);
  digitalWrite(PIN_ENABLE_I2C_PULLUP, HIGH);
  delay(2);

  Wire1.begin();
  delay(1);

  // LSM9DS1 AG WHO_AM_I @ 0x6B reg 0x0F → 0x68
  bool ag = whoAmI(Wire1, 0x6B, 0x0F, 0x68);
  // LSM9DS1 Mag WHO_AM_I_M @ 0x1E reg 0x0F → 0x3D
  bool mag = whoAmI(Wire1, 0x1E, 0x0F, 0x3D);
  // HTS221 WHO_AM_I @ 0x5F reg 0x0F → 0xBC
  bool hts = whoAmI(Wire1, 0x5F, 0x0F, 0xBC);
  // LPS22HB WHO_AM_I @ 0x5C reg 0x0F → 0xB1
  bool lps = whoAmI(Wire1, 0x5C, 0x0F, 0xB1);
  // APDS9960 ID @ 0x39 reg 0x92 → 0xAB
  bool apds = whoAmI(Wire1, 0x39, 0x92, 0xAB);

  if (ag && mag && hts && lps && apds) {
    logLine("LW_SENSE_OK");
  } else {
    char buf[64];
    snprintf(buf, sizeof(buf), "LW_SENSE_FAIL ag=%u mag=%u hts=%u lps=%u apds=%u",
             (unsigned)ag, (unsigned)mag, (unsigned)hts, (unsigned)lps, (unsigned)apds);
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
