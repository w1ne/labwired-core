// Stock STM32duino `analogWrite()` hardware PWM on the STM32F103C8T6 Blue Pill.
//
// PA6 is TIM3_CH1 and drives the ENA input of an L298N motor driver; PB12/PB13
// are IN1/IN2. PA1 is TIM2_CH2, kept as an independent second channel on a
// second timer so a test can show two duties held at the same time.
//
// Nothing here touches a register: `analogWriteFrequency()` and
// `analogWrite()` are the library calls a Blue Pill sketch uses.

#include <Arduino.h>

static void report(const char *tag, int value) {
  Serial.print(tag);
  Serial.print(' ');
  Serial.println(value);
}

void setup() {
  Serial.begin(115200);

  // L298N channel A: IN1 high, IN2 low is "forward".
  pinMode(PB12, OUTPUT);
  pinMode(PB13, OUTPUT);
  digitalWrite(PB12, HIGH);
  digitalWrite(PB13, LOW);

  // 1 kHz carrier, then 64/255 on PA6 and 192/255 on PA1 (8-bit resolution).
  analogWriteFrequency(1000);
  analogWrite(PA6, 64);
  analogWrite(PA1, 192);

  Serial.println("F103PWM READY");
  report("PA6_DUTY", 64);
  report("PA1_DUTY", 192);
}

void loop() {
  delay(100);

  // The same channel at 0 %: the pin must go low and stay low.
  analogWrite(PA6, 0);
  Serial.println("F103PWM STOP");
  report("PA6_DUTY", 0);

  for (;;) {
  }
}
