#include <Arduino.h>
#include <esp_rom_sys.h>
extern "C" void esp_crosscore_int_send_print_backtrace(int core_id);
extern "C" void esp_crosscore_int_send_yield(int core_id);
static volatile uint32_t g_spin = 0; static volatile int g_phase = 0;
static void mon(void *) {
  for (int i = 0;; i++) {
    esp_rom_printf("MON i=%d spin=%u\n", i, g_spin);
    if (i == 4) { esp_rom_printf("SEND_YIELD\n"); esp_crosscore_int_send_yield(1); }
    if (i == 8) { esp_rom_printf("SEND_BT\n"); esp_crosscore_int_send_print_backtrace(1); }
    vTaskDelay(20);
  }
}
void setup() {
  Serial.begin(115200);
  xTaskCreatePinnedToCore(mon, "mon", 4096, NULL, 5, NULL, 0);
  for (;;) { g_spin++; }
}
void loop() {}
