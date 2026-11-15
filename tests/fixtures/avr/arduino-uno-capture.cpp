// Arduino Uno sketch for the Timer1 input capture and temperature sensor twin.
//
// Built with PlatformIO `atmelavr` (board = uno, framework = arduino):
//   pio run   (copy .pio/build/uno/firmware.elf to arduino-uno-capture.elf)
//
// 1. The ADC temperature sensor (mux 0x08) against the internal 1.1 V.
// 2. Polled capture: D8 (ICP1) driven as an output, rising edge, clk/8.
// 3. Interrupt capture: tone(8, 1000) toggles D8 from the Timer2 ISR, and
//    TIMER1_CAPT records eight rising edges at 0.5 us per tick.
#include <Arduino.h>

volatile uint16_t caps[8];
volatile uint8_t ncap = 0;
ISR(TIMER1_CAPT_vect) {
  if (ncap < 8) caps[ncap++] = ICR1;
}

static uint16_t adc_raw(uint8_t admux) {
  ADMUX = admux;
  delay(3);
  ADCSRA |= _BV(ADSC);
  while (ADCSRA & _BV(ADSC)) {
  }
  return ADC;
}

void setup() {
  Serial.begin(115200);
  analogRead(A0);
  Serial.print("TEMP raw=");
  Serial.println(adc_raw(_BV(REFS1) | _BV(REFS0) | 0x08));

  pinMode(8, OUTPUT);
  digitalWrite(8, LOW);
  TCCR1A = 0;
  TCCR1B = _BV(ICES1) | _BV(CS11);
  TCNT1 = 0;
  TIFR1 = _BV(ICF1);
  delayMicroseconds(200);
  digitalWrite(8, HIGH);
  uint8_t icf = (TIFR1 & _BV(ICF1)) ? 1 : 0;
  uint16_t icr = ICR1;
  Serial.print("POLL icf=");
  Serial.print(icf);
  Serial.print(" icr1=");
  Serial.println(icr);

  TIFR1 = _BV(ICF1);
  TIMSK1 = _BV(ICIE1);
  tone(8, 1000);
  uint32_t t0 = millis();
  while (ncap < 8 && millis() - t0 < 50) {
  }
  noTone(8);
  TIMSK1 = 0;
  Serial.print("ISR n=");
  Serial.print(ncap);
  for (uint8_t i = 1; i < ncap; i++) {
    Serial.print(i == 1 ? " periods_us=" : ",");
    Serial.print((uint16_t)(caps[i] - caps[i - 1]) / 2);
  }
  Serial.println();
  Serial.println("DONE");
}

void loop() {}
