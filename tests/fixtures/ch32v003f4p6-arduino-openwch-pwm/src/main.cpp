// Official openwch Arduino core (arduino_core_ch32) on CH32V003F4P6.
// analogWrite() on PD4 (TIM2_CH1) and PD2 (TIM1_CH1). The core's default
// PWM_FREQUENCY is 1000 Hz and PWM_RESOLUTION is 12, so 2048 is 50 % and
// 1024 is 25 % of the 12-bit range. pwm_start() enables the timer clock,
// sets PWM mode 1 with preload, writes PSC/ARR/CCR and starts the counter;
// pinmap_pinout() switches the pad to alternate-function push-pull.
#include <Arduino.h>

void setup() {
  Serial.begin(115200);
  Serial.println("CH32V003_PWM_BOOT");
  analogWrite(PD4, 2048);
  analogWrite(PD2, 1024);
  Serial.println("CH32V003_PWM_SET");
  Serial.println("CH32V003_PWM_DONE");
}

void loop() {}
