// Marketplace sensor review — ESP32-C3 Super Mini (Arduino + Wire)
// Reads popular I2C modules and prints lines the stimuli API can drive.
//
// I2C: SDA=GPIO4, SCL=GPIO5 (same as LabWired C3 OLED labs)
//
// Super Mini builds with ARDUINO_USB_CDC_ON_BOOT=1, so `Serial` is USB-CDC.
// LabWired captures hardware UART0 — use Serial0 (or dual-print) for sim.

#include <Arduino.h>
#include <Wire.h>

static constexpr int PIN_SDA = 4;
static constexpr int PIN_SCL = 5;

// Super Mini: Serial = USB-CDC, Serial0 = UART0 (what LabWired captures).
// Always dual-init; always log on UART0 so rom-boot sims see the sketch.
static void mktSerialBegin() {
  Serial.begin(115200);
#if defined(ARDUINO_USB_CDC_ON_BOOT) && (ARDUINO_USB_CDC_ON_BOOT)
  Serial.setTxTimeoutMs(0);  // do not block if no USB host (simulator)
#endif
  Serial0.begin(115200);
}

static uint16_t i2cRead16BE(uint8_t addr7, uint8_t reg) {
  Wire.beginTransmission(addr7);
  Wire.write(reg);
  if (Wire.endTransmission(false) != 0) return 0xFFFF;
  if (Wire.requestFrom((int)addr7, 2) != 2) return 0xFFFF;
  uint16_t hi = Wire.read();
  uint16_t lo = Wire.read();
  return (hi << 8) | lo;
}

static uint8_t i2cRead8(uint8_t addr7, uint8_t reg) {
  Wire.beginTransmission(addr7);
  Wire.write(reg);
  if (Wire.endTransmission(false) != 0) return 0xFF;
  if (Wire.requestFrom((int)addr7, 1) != 1) return 0xFF;
  return Wire.read();
}

static void readIna219() {
  // bus voltage reg 0x02 — bits 15:3 are 4 mV counts
  uint16_t bus = i2cRead16BE(0x40, 0x02);
  uint16_t cur = i2cRead16BE(0x40, 0x04);
  uint32_t mv = ((bus >> 3) & 0x1FFF) * 4u;
  int32_t ma = (int16_t)cur / 10;
  Serial0.print("INA219 Vbus_mV=");
  Serial0.print(mv);
  Serial0.print(" I_mA=");
  Serial0.println(ma);
}

static void readAds1115() {
  // assume already configured; read conversion
  int16_t raw = (int16_t)i2cRead16BE(0x48, 0x00);
  Serial0.print("ADS1115 A0_raw=");
  Serial0.println(raw);
}

static void writeAdsConfig() {
  // OS + MUX AIN0 SE + PGA ±4.096 + continuous
  Wire.beginTransmission(0x48);
  Wire.write(0x01);
  Wire.write(0xC3);
  Wire.write(0x83);
  Wire.endTransmission();
}

static void readDs3231() {
  uint8_t sec = i2cRead8(0x68, 0x00) & 0x7F;
  uint8_t min = i2cRead8(0x68, 0x01) & 0x7F;
  uint8_t hour = i2cRead8(0x68, 0x02) & 0x3F;
  auto bcd = [](uint8_t v) { return (v >> 4) * 10 + (v & 0x0F); };
  Serial0.print("DS3231 TIME=");
  if (bcd(hour) < 10) Serial0.print('0');
  Serial0.print(bcd(hour));
  Serial0.print(':');
  if (bcd(min) < 10) Serial0.print('0');
  Serial0.print(bcd(min));
  Serial0.print(':');
  if (bcd(sec) < 10) Serial0.print('0');
  Serial0.println(bcd(sec));
}

static void readAs5600() {
  uint16_t raw = i2cRead16BE(0x36, 0x0C) & 0x0FFF;
  // 0..4095 → degrees
  uint32_t deg = (raw * 360u) / 4096u;
  Serial0.print("AS5600 angle_deg=");
  Serial0.println(deg);
}

static void readBno055() {
  // chip id
  uint8_t id = i2cRead8(0x28, 0x00);
  int16_t h = (int16_t)(i2cRead8(0x28, 0x1A) | (i2cRead8(0x28, 0x1B) << 8));
  // degrees * 16
  Serial0.print("BNO055 chip=");
  Serial0.print(id, HEX);
  Serial0.print(" heading=");
  Serial0.println(h / 16);
}

static void readVl53l0x() {
  // model id then range high/low (simplified)
  uint8_t mid = i2cRead8(0x29, 0xC0);
  uint16_t mm = ((uint16_t)i2cRead8(0x29, 0x1E) << 8) | i2cRead8(0x29, 0x1F);
  Serial0.print("VL53L0X id=");
  Serial0.print(mid, HEX);
  Serial0.print(" dist_mm=");
  Serial0.println(mm);
}

void setup() {
  mktSerialBegin();
  delay(50);
  Serial0.println("MARKETPLACE ARDUINO C3");
  Wire.begin(PIN_SDA, PIN_SCL);
  Wire.setClock(100000);
  writeAdsConfig();
  // BNO055 NDOF mode
  Wire.beginTransmission(0x28);
  Wire.write(0x3D);
  Wire.write(0x0C);
  Wire.endTransmission();
  // VL53L0X start ranging
  Wire.beginTransmission(0x29);
  Wire.write(0x00);
  Wire.write(0x01);
  Wire.endTransmission();
  Serial0.println("SENSORS READY");
}

void loop() {
  readIna219();
  readAds1115();
  readDs3231();
  readAs5600();
  readBno055();
  readVl53l0x();
  Serial0.println("---");
  delay(200);
}
