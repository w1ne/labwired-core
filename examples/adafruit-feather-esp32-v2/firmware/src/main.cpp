#include <Arduino.h>
#include <driver/gpio.h>

namespace {
constexpr gpio_num_t kUserLed = GPIO_NUM_13;
}

void setup() {
  gpio_reset_pin(kUserLed);
  gpio_set_direction(kUserLed, GPIO_MODE_OUTPUT);
  gpio_set_level(kUserLed, 0);
}

void loop() {
  static bool led_high = false;
  led_high = !led_high;
  gpio_set_level(kUserLed, led_high ? 1 : 0);
}
