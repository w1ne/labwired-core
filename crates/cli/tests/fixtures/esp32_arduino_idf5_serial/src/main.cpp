// Serial.begin() installs the IDF UART driver, whose tx_mux comes from
// xSemaphoreCreateMutexWithCaps -> xQueueCreateMutexStatic. If the sim fakes
// that create, the first print spins forever on an uninitialised spinlock.
#include <Arduino.h>
void setup() {
  Serial.begin(115200);
  pinMode(4, OUTPUT);
  Serial.println("idf5-boot");
}
void loop() {
  static bool s = false;
  s = !s;
  digitalWrite(4, s);
  Serial.println("tick");
  delay(100);
}
