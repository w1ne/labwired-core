#include <Arduino.h>

// D9 = PB0 = TCA0 WO0, D3 = PF5 = TCB1 WO: the two timer kinds behind
// analogWrite() on the Nano Every.
static const uint8_t kTcaPin = 9;
static const uint8_t kTcbPin = 3;

void setup() {
  Serial.begin(115200);
  analogWrite(kTcaPin, 64);   // 25 % duty
  analogWrite(kTcbPin, 192);  // 75 % duty
  Serial.println("NANO_EVERY_PWM");
  delay(20);
  analogWrite(kTcaPin, 128);  // 50 % duty, taken at the next period
  Serial.println("PWM_HALF");
  delay(20);
  digitalWrite(kTcbPin, LOW);  // stops the PWM on D3
  Serial.println("PWM_D3_OFF");
}

void loop() {}
