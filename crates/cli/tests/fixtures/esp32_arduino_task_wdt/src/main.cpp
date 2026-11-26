// Arduino-ESP32 task watchdog on a stuck loopTask (core 1).
//
// The TWDT runs on TIMG0's main-system watchdog. It must stay quiet while the
// task feeds it and fire once the task stops. With trigger_panic = false the
// TWDT ISR on core 0 logs the starving task and sends core 1 a cross-core
// "print backtrace" request; core 1 must service it and resume loopTask.
#include <Arduino.h>
#include <esp_task_wdt.h>

static volatile bool g_fired = false;

extern "C" void esp_task_wdt_isr_user_handler(void) { g_fired = true; }

void setup() {
  Serial.begin(115200);
  esp_task_wdt_config_t cfg = {
      .timeout_ms = 100, .idle_core_mask = 0, .trigger_panic = false};
  esp_task_wdt_reconfigure(&cfg);
  esp_task_wdt_add(NULL);
  Serial.println("LW_TWDT_ARM");

  // Feed for three timeout periods: a model that ignores feeds fires here.
  uint32_t t = millis();
  while (millis() - t < 300) {
    esp_task_wdt_reset();
    delayMicroseconds(1000);
  }
  Serial.println(g_fired ? "LW_TWDT_FALSE_FIRE" : "LW_TWDT_FED_OK");

  // Stop feeding. The ISR sets g_fired; loopTask only sees it if core 1
  // came back from the backtrace request.
  t = millis();
  while (!g_fired && millis() - t < 1000) {
  }
  Serial.println(g_fired ? "LW_TWDT_FIRED" : "LW_TWDT_MISSING");
  esp_task_wdt_delete(NULL);
}

void loop() { delay(1000); }
