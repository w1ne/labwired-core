// Official openwch Arduino core (arduino_core_ch32) on CH32V003F4P6.
// analogRead() on PA2 (ADC1_IN0) and PD4 (ADC1_IN7): the core enables the
// ADC clock, writes RSQR3 rank 1 and SAMPTR, sets SWSTART and polls STATR.EOC,
// then returns RDATAR. The CH32V003 ADC is 10 bits (variant ADC_RESOLUTION).
#include <Arduino.h>

void setup() {
  Serial.begin(115200);
  Serial.println("CH32V003_ADC_BOOT");
  uint32_t a0 = analogRead(PA2);
  uint32_t a7 = analogRead(PD4);
  Serial.print("pa2=");
  Serial.println(a0);
  Serial.print("pd4=");
  Serial.println(a7);
  Serial.println(a0 <= 1023 && a7 <= 1023 && a0 < a7 ? "CH32V003_ADC_PASS" : "CH32V003_ADC_FAIL");
  Serial.println("CH32V003_ADC_DONE");
}

void loop() {}
