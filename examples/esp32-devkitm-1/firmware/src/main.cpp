#include <Arduino.h>
#include <driver/gpio.h>

namespace {
constexpr gpio_num_t kHeaderLedPin = GPIO_NUM_23;
}

void setup() {
  Serial.begin(115200);
  Serial.println("ESP32_DEVKITM1_UART0_OK");

  gpio_reset_pin(kHeaderLedPin);
  gpio_set_direction(kHeaderLedPin, GPIO_MODE_OUTPUT);
  gpio_set_level(kHeaderLedPin, 0);
}

void loop() {
  static bool led_high = false;
  led_high = !led_high;
  gpio_set_level(kHeaderLedPin, led_high ? 1 : 0);
}
