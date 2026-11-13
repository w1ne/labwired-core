// Realtime-factor workload: ESP32-C3 (arduino-esp32, Wi-Fi never started) +
// INA219 on I2C (GPIO4/GPIO5) read every 50 ms and logged over UART0.
// Runs forever; the gate stops it on a fixed cycle budget.
#include <Wire.h>

static uint16_t readReg(uint8_t reg) {
  Wire.beginTransmission(0x40);
  Wire.write(reg);
  Wire.endTransmission();
  Wire.requestFrom(0x40, 2);
  uint16_t v = Wire.read() << 8;
  v |= Wire.read();
  return v;
}

unsigned long lastRead = 0;
unsigned int sample = 0;

void setup() {
  Serial.begin(115200);
  Wire.begin(4, 5);
  Serial.println("RTF_READY c3_i2c_uart");
}

void loop() {
  unsigned long now = millis();
  if (now - lastRead >= 50) {
    lastRead = now;
    uint16_t bus = readReg(0x02);
    Serial.printf("s=%u bus=0x%04x\n", ++sample, bus);
  }
}
