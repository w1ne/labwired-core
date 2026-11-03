#include <Arduino.h>
#include <driver/gpio.h>

void setup() {
  gpio_set_direction(GPIO_NUM_46, GPIO_MODE_OUTPUT); // RGB red, active low
  gpio_set_level(GPIO_NUM_46, 0);
}

void loop() {
  delay(1000);
}
