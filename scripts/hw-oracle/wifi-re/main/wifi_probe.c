// Minimal ESP32-C3 radio-init probe: bring the WiFi driver up to the point
// where PHY + MAC registers are configured, then idle. No connect/scan — keeps
// the JTAG MMIO trace focused on phy_enable + esp_wifi_init/start.
#include "nvs_flash.h"
#include "esp_netif.h"
#include "esp_event.h"
#include "esp_wifi.h"
#include "esp_log.h"

static const char *TAG = "probe";

// Trace anchors — set JTAG breakpoints on these symbols.
void __attribute__((noinline)) probe_before_init(void) { __asm__ volatile("nop"); }
void __attribute__((noinline)) probe_after_init(void)  { __asm__ volatile("nop"); }
// Tight bracket around esp_wifi_start(): the busy-wait *poll surface* (the MAC
// status bits the driver spins on while bringing the MAC + DMA rings up) lives
// between these two anchors. trace_poll.sh arms a read-watchpoint over the MAC
// window only across this bracket, so the capture isn't drowned out by the
// config-write surface that trace_radio.sh already recovered.
void __attribute__((noinline)) probe_start_enter(void) { __asm__ volatile("nop"); }
void __attribute__((noinline)) probe_after_start(void) { __asm__ volatile("nop"); }
void __attribute__((noinline)) probe_idle(void)        { __asm__ volatile("nop"); }

void app_main(void)
{
    ESP_ERROR_CHECK(nvs_flash_init());
    ESP_ERROR_CHECK(esp_netif_init());
    ESP_ERROR_CHECK(esp_event_loop_create_default());
    esp_netif_create_default_wifi_sta();

    wifi_init_config_t cfg = WIFI_INIT_CONFIG_DEFAULT();
    probe_before_init();
    ESP_ERROR_CHECK(esp_wifi_init(&cfg));          // allocates MAC, phy_enable
    probe_after_init();
    ESP_ERROR_CHECK(esp_wifi_set_mode(WIFI_MODE_STA));
    probe_start_enter();
    ESP_ERROR_CHECK(esp_wifi_start());             // MAC/DMA ring + PHY RF on
    probe_after_start();
    ESP_LOGI(TAG, "wifi up; idling for trace");
    probe_idle();
    while (1) { vTaskDelay(1000); }
}
