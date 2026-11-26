#include <Arduino.h>
#include <esp_task_wdt.h>
#include <esp_rom_sys.h>
static volatile bool g_fired = false;
static volatile uint32_t g_spin = 0, g_isr = 0;
extern "C" void esp_task_wdt_isr_user_handler(void) { g_fired = true; g_isr++; }
static void mon(void *) {
  for (;;) { esp_rom_printf("MON spin=%u isr=%u fired=%d\n", g_spin, g_isr, g_fired); vTaskDelay(25); }
}
void setup() {
  Serial.begin(115200);
  esp_task_wdt_config_t cfg = {.timeout_ms = 100, .idle_core_mask = 0, .trigger_panic = false};
  esp_task_wdt_reconfigure(&cfg);
  esp_task_wdt_add(NULL);
  xTaskCreatePinnedToCore(mon, "mon", 4096, NULL, 5, NULL, 0);
  Serial.println("LW_TWDT_ARM");
  uint32_t t = millis();
  while (!g_fired && millis() - t < 1000) { g_spin++; }
  esp_rom_printf("ROM_%s\n", g_fired ? "FIRED" : "MISSING");
}
void loop() { delay(1000); }
