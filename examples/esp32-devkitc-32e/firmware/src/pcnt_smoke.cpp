#include <Arduino.h>
#include "driver/pcnt.h"

namespace {
constexpr gpio_num_t kPulsePin = GPIO_NUM_23;
constexpr gpio_num_t kPassPin = GPIO_NUM_16;
constexpr pcnt_unit_t kUnit = PCNT_UNIT_0;
constexpr int16_t kPulseCount = 8;

bool configure_counter() {
  pcnt_config_t config = {};
  config.pulse_gpio_num = kPulsePin;
  config.ctrl_gpio_num = PCNT_PIN_NOT_USED;
  config.channel = PCNT_CHANNEL_0;
  config.unit = kUnit;
  config.pos_mode = PCNT_COUNT_INC;
  config.neg_mode = PCNT_COUNT_DIS;
  config.lctrl_mode = PCNT_MODE_KEEP;
  config.hctrl_mode = PCNT_MODE_KEEP;
  config.counter_h_lim = 1000;
  config.counter_l_lim = -1000;

  return pcnt_unit_config(&config) == ESP_OK;
}
}  // namespace

void setup() {
  Serial.begin(115200);
  pinMode(static_cast<uint8_t>(kPassPin), OUTPUT);
  digitalWrite(static_cast<uint8_t>(kPassPin), LOW);

  if (!configure_counter()) {
    Serial.println("ESP32_DEVKITC32E_PCNT_CONFIG_FAILED");
    return;
  }

  // pcnt_unit_config configures its signal pin as an input. Re-enable the
  // pad's output buffer afterward; the input-matrix route remains selected.
  pinMode(static_cast<uint8_t>(kPulsePin), OUTPUT);
  digitalWrite(static_cast<uint8_t>(kPulsePin), LOW);
  if (pcnt_counter_pause(kUnit) != ESP_OK ||
      pcnt_counter_clear(kUnit) != ESP_OK ||
      pcnt_counter_resume(kUnit) != ESP_OK) {
    Serial.println("ESP32_DEVKITC32E_PCNT_START_FAILED");
    return;
  }

  // PCNT's GPIO-matrix input selects the same pad the sketch drives. Use a
  // 100 us high/low period so this also clears the real chip's glitch filter.
  for (int16_t i = 0; i < kPulseCount; ++i) {
    digitalWrite(static_cast<uint8_t>(kPulsePin), HIGH);
    delayMicroseconds(100);
    digitalWrite(static_cast<uint8_t>(kPulsePin), LOW);
    delayMicroseconds(100);
  }

  int16_t count = 0;
  if (pcnt_get_counter_value(kUnit, &count) != ESP_OK) {
    Serial.println("ESP32_DEVKITC32E_PCNT_READ_FAILED");
  } else if (count == kPulseCount) {
    // GPIO16 / J3 pin 12 is a simple pass indicator for the hosted oracle.
    digitalWrite(static_cast<uint8_t>(kPassPin), HIGH);
    Serial.printf("ESP32_DEVKITC32E_PCNT_UNIT0=%d\n", count);
  } else {
    Serial.printf("ESP32_DEVKITC32E_PCNT_UNIT0=%d_EXPECTED=%d\n", count,
                  kPulseCount);
  }
}

void loop() {}
