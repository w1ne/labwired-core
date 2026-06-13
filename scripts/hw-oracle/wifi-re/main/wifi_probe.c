// ESP32-C3 WiFi bring-up + connect probe. Brings the WiFi driver up and then
// attempts to associate with an AP, so the REAL MAC transmits the scan/auth/
// assoc frame sequence through its DMA rings — the traffic the LabWired MAC <->
// SimNet bridge consumes. (Originally a radio-init-only probe; extended to
// connect for the bridge work — see docs/esp32c3_wifi_mac_bridge.md.)
#include "nvs_flash.h"
#include "esp_netif.h"
#include "esp_event.h"
#include "esp_wifi.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include <string.h>

static const char *TAG = "probe";

#define PROBE_SSID "labwired-ap"
// OPEN auth (no password) — the bridge's first comms milestone associates with
// an OPEN AP, avoiding the WPA2 4-way handshake.
#define PROBE_PASS ""

// Trace anchors — set JTAG/sim breakpoints on these symbols.
void __attribute__((noinline)) probe_before_init(void) { __asm__ volatile("nop"); }
void __attribute__((noinline)) probe_after_init(void)  { __asm__ volatile("nop"); }
void __attribute__((noinline)) probe_start_enter(void) { __asm__ volatile("nop"); }
void __attribute__((noinline)) probe_after_start(void) { __asm__ volatile("nop"); }
void __attribute__((noinline)) probe_connect_enter(void) { __asm__ volatile("nop"); }
void __attribute__((noinline)) probe_idle(void)        { __asm__ volatile("nop"); }

static void wifi_event_handler(void *arg, esp_event_base_t base,
                               int32_t id, void *data)
{
    if (base == WIFI_EVENT && id == WIFI_EVENT_STA_START) {
        ESP_LOGI(TAG, "sta start -> connect");
        probe_connect_enter();
        esp_wifi_connect();
    } else if (base == WIFI_EVENT && id == WIFI_EVENT_STA_CONNECTED) {
        ESP_LOGI(TAG, "STA CONNECTED");
    } else if (base == WIFI_EVENT && id == WIFI_EVENT_STA_DISCONNECTED) {
        ESP_LOGI(TAG, "sta disconnected -> retry");
        esp_wifi_connect();
    } else if (base == IP_EVENT && id == IP_EVENT_STA_GOT_IP) {
        ESP_LOGI(TAG, "GOT IP");
    }
}

void app_main(void)
{
    ESP_ERROR_CHECK(nvs_flash_init());
    ESP_ERROR_CHECK(esp_netif_init());
    ESP_ERROR_CHECK(esp_event_loop_create_default());
    esp_netif_create_default_wifi_sta();

    ESP_ERROR_CHECK(esp_event_handler_instance_register(
        WIFI_EVENT, ESP_EVENT_ANY_ID, &wifi_event_handler, NULL, NULL));
    ESP_ERROR_CHECK(esp_event_handler_instance_register(
        IP_EVENT, IP_EVENT_STA_GOT_IP, &wifi_event_handler, NULL, NULL));

    wifi_init_config_t cfg = WIFI_INIT_CONFIG_DEFAULT();
    probe_before_init();
    ESP_ERROR_CHECK(esp_wifi_init(&cfg));
    probe_after_init();

    wifi_config_t wc = { 0 };
    strncpy((char *)wc.sta.ssid, PROBE_SSID, sizeof(wc.sta.ssid));
    strncpy((char *)wc.sta.password, PROBE_PASS, sizeof(wc.sta.password));
    ESP_ERROR_CHECK(esp_wifi_set_mode(WIFI_MODE_STA));
    ESP_ERROR_CHECK(esp_wifi_set_config(WIFI_IF_STA, &wc));
    probe_start_enter();
    ESP_ERROR_CHECK(esp_wifi_start());   // STA start event -> esp_wifi_connect()
    probe_after_start();
    ESP_LOGI(TAG, "wifi up; connecting to %s", PROBE_SSID);
    probe_idle();
    while (1) { vTaskDelay(1000); }
}
