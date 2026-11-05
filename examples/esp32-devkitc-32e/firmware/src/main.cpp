#include <Arduino.h>
#include <driver/gpio.h>

namespace {
constexpr gpio_num_t kHeaderPin = GPIO_NUM_23;
}

void setup() {
  gpio_reset_pin(kHeaderPin);
  gpio_set_direction(kHeaderPin, GPIO_MODE_OUTPUT);
  gpio_set_level(kHeaderPin, 0);
}

void loop() {
  static bool output_high = false;
  output_high = !output_high;
  gpio_set_level(kHeaderPin, output_high ? 1 : 0);
}
