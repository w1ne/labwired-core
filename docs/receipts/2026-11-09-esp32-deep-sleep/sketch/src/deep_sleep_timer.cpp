// Stock Arduino-ESP32 DeepSleep/TimerWakeUp pattern: RTC_DATA_ATTR counter,
// timer wake-up, esp_deep_sleep_start(). Prints LW_DS_OK after the third boot.
#include <Arduino.h>
#include "esp_sleep.h"

RTC_DATA_ATTR int bootCount = 0;

void setup() {
  Serial.begin(115200);
  ++bootCount;
  esp_sleep_wakeup_cause_t cause = esp_sleep_get_wakeup_cause();
  Serial.printf("boot=%d cause=%d reset=%d\n", bootCount, (int)cause,
                (int)esp_reset_reason());
  if (bootCount >= 3) {
    if (cause == ESP_SLEEP_WAKEUP_TIMER &&
        esp_reset_reason() == ESP_RST_DEEPSLEEP) {
      Serial.println("LW_DS_OK");
    } else {
      Serial.println("LW_DS_FAIL");
    }
    return;
  }
  esp_sleep_enable_timer_wakeup(50 * 1000ULL);  // 50 ms
  Serial.println("sleeping");
  Serial.flush();
  esp_deep_sleep_start();
  Serial.println("LW_DS_FAIL not asleep");
}

void loop() { delay(1000); }
