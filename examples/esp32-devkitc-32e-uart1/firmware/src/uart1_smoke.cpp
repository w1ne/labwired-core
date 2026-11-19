#include <Arduino.h>

constexpr int kUart1TxPin = 16;  // J3 pin 12 on ESP32-DevKitC V4.

void setup() {
  Serial.begin(115200);
  Serial1.begin(115200, SERIAL_8N1, -1, kUart1TxPin);
  Serial1.print("UUUUUUUU\n");
  Serial1.flush();
  Serial.println("ESP32_DEVKITC32E_UART1_TX_OK");
}

void loop() { delay(1000); }
