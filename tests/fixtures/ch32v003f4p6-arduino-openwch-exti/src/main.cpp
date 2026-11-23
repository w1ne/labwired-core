// Official openwch Arduino core (arduino_core_ch32) on CH32V003F4P6.
// attachInterrupt on PD3 (falling, pull-up) runs through AFIO EXTICR, EXTI
// line 3 and EXTI7_0_IRQn (vector 20, HPE fast handler); Serial.read() polls
// USART1 RXNE on PD6.
#include <Arduino.h>

volatile uint32_t falls = 0;

void onFall() { falls++; }

void setup() {
  Serial.begin(115200);
  attachInterrupt(PD3, GPIO_Mode_IPU, onFall, EXTI_Mode_Interrupt, EXTI_Trigger_Falling);
  Serial.println("CH32V003_EXTI_BOOT");
  uint32_t t0 = millis();
  while (falls < 3 && millis() - t0 < 20) {
  }
  Serial.print("falls=");
  Serial.println(falls);
  char buf[16];
  uint8_t n = 0;
  t0 = millis();
  while (n < sizeof(buf) - 1 && millis() - t0 < 20) {
    int c = Serial.read();
    if (c < 0) continue;
    if (c == '\n') break;
    buf[n++] = (char)c;
  }
  buf[n] = 0;
  Serial.print("rx=");
  Serial.println(buf);
  Serial.println(falls == 3 && strcmp(buf, "wch") == 0 ? "CH32V003_EXTI_PASS" : "CH32V003_EXTI_FAIL");
  Serial.println("CH32V003_EXTI_DONE");
}

void loop() {}
