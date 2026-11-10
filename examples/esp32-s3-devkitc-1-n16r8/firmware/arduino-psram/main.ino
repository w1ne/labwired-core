#include <Arduino.h>
#include <esp_heap_caps.h>

void setup() {
  Serial.begin(115200);
  if (!psramFound()) {
    Serial.println("ESP32_S3_N16R8_PSRAM_NOT_FOUND");
    return;
  }

  const size_t bytes = 4096;
  volatile uint32_t *buffer = (volatile uint32_t *)heap_caps_malloc(
      bytes, MALLOC_CAP_SPIRAM | MALLOC_CAP_8BIT);
  if (buffer == nullptr) {
    Serial.println("ESP32_S3_N16R8_PSRAM_ALLOC_FAILED");
    return;
  }

  for (size_t i = 0; i < bytes / sizeof(uint32_t); ++i) {
    buffer[i] = 0xA5C30000u ^ (uint32_t)i;
  }
  for (size_t i = 0; i < bytes / sizeof(uint32_t); ++i) {
    if (buffer[i] != (0xA5C30000u ^ (uint32_t)i)) {
      Serial.println("ESP32_S3_N16R8_PSRAM_VERIFY_FAILED");
      heap_caps_free((void *)buffer);
      return;
    }
  }

  Serial.printf("ESP32_S3_N16R8_PSRAM_PASS bytes=%u\n", (unsigned)bytes);
  heap_caps_free((void *)buffer);
}

void loop() {
  delay(1000);
}
