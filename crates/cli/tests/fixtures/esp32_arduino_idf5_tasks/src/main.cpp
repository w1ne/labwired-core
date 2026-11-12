// Round 287: two FreeRTOS tasks, real vTaskDelay, shared Serial (UART tx_mux).
#include <Arduino.h>
static void worker(void *) {
  for (unsigned n = 0;; n++) {
    Serial.printf("w %u\n", n);
    vTaskDelay(1);
  }
}
void setup() {
  Serial.begin(115200);
  pinMode(4, OUTPUT);
  Serial.println("tt-boot");
  xTaskCreatePinnedToCore(worker, "worker", 4096, nullptr, 1, nullptr, 0);
}
void loop() {
  static unsigned n = 0;
  digitalWrite(4, n & 1);
  Serial.printf("m %u\n", n++);
  vTaskDelay(1);
}
