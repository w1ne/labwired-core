#include <Arduino.h>
#include <driver/gpio.h>

void setup() {
  gpio_set_direction(GPIO_NUM_46, GPIO_MODE_OUTPUT); // RGB red, active low
  gpio_set_direction(GPIO_NUM_0, GPIO_MODE_OUTPUT);  // standard RGB green contact
  gpio_set_direction(GPIO_NUM_45, GPIO_MODE_OUTPUT); // standard RGB blue contact
  gpio_set_level(GPIO_NUM_46, 1); // RGB red off
  gpio_set_level(GPIO_NUM_0, 1);  // RGB GPIO0 channel off
  gpio_set_level(GPIO_NUM_45, 1); // RGB GPIO45 channel off
  gpio_set_level(GPIO_NUM_46, 0);
  gpio_set_level(GPIO_NUM_0, 0);
  gpio_set_level(GPIO_NUM_45, 0);
}

void loop() {
  delay(1000);
}
