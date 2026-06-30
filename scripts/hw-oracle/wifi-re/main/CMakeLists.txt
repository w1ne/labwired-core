idf_component_register(SRCS "wifi_probe.c" INCLUDE_DIRS "."
                       REQUIRES nvs_flash esp_wifi esp_netif esp_event lwip)
