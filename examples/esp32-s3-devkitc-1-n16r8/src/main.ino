#include <Arduino.h>

void setup() {
  Serial0.begin(115200);
  Serial0.println("ESP32_S3_DEVKITC1_N16R8_UART0_OK");
}

void loop() {
  delay(1000);
}
