// Stock esp_light_sleep_start() with a timer wake-up: execution resumes
// after the call, the cause is the timer, and RAM state is kept.
#include <Arduino.h>
#include "esp_sleep.h"

int counter = 0;

void setup() {
  Serial.begin(115200);
  for (int i = 0; i < 3; i++) {
    esp_sleep_enable_timer_wakeup(20 * 1000ULL);  // 20 ms
    Serial.flush();
    esp_err_t err = esp_light_sleep_start();
    counter++;
    Serial.printf("wake=%d err=%d cause=%d\n", counter, (int)err,
                  (int)esp_sleep_get_wakeup_cause());
  }
  if (counter == 3 && esp_sleep_get_wakeup_cause() == ESP_SLEEP_WAKEUP_TIMER) {
    Serial.println("LW_LS_OK");
  } else {
    Serial.println("LW_LS_FAIL");
  }
}

void loop() { delay(1000); }
