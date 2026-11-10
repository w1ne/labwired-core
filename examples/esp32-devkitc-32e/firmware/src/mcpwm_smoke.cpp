#include <Arduino.h>
#include "driver/mcpwm.h"
#include "driver/gpio.h"

constexpr gpio_num_t kServoSignalPin = GPIO_NUM_23;  // J3 pin 2.
constexpr float kCenterDutyPercent = 7.5f;           // 1.5 ms in a 20 ms frame.

void setup() {
  Serial.begin(115200);

  if (mcpwm_gpio_init(MCPWM_UNIT_0, MCPWM0A, kServoSignalPin) != ESP_OK) {
    Serial.println("ESP32_DEVKITC32E_MCPWM_GPIO_INIT_FAILED");
    return;
  }

  mcpwm_config_t config = {};
  config.frequency = 50;
  config.cmpr_a = 0;
  config.cmpr_b = 0;
  config.counter_mode = MCPWM_UP_COUNTER;
  config.duty_mode = MCPWM_DUTY_MODE_0;
  if (mcpwm_init(MCPWM_UNIT_0, MCPWM_TIMER_0, &config) != ESP_OK) {
    Serial.println("ESP32_DEVKITC32E_MCPWM_INIT_FAILED");
    return;
  }

  if (mcpwm_set_duty(MCPWM_UNIT_0, MCPWM_TIMER_0, MCPWM_OPR_A,
                     kCenterDutyPercent) != ESP_OK) {
    Serial.println("ESP32_DEVKITC32E_MCPWM_DUTY_FAILED");
    return;
  }

  // The marker follows the legacy ESP-IDF MCPWM setup and 50 Hz center command.
  Serial.println("ESP32_DEVKITC32E_MCPWM_SERVO_OK");
}

void loop() {}
