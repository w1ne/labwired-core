// Arduino Mega 2560 sketch for the console tap: Serial (USART0) carries the
// console through the USB bridge; Serial1 and Serial2 are pin-level links
// (D18/D19, D16/D17) that a run must NOT splice into the console capture.
// Built with PlatformIO atmelavr 5.3.0, framework-arduino-avr 5.4.0
// (ArduinoCore-avr), board megaatmega2560:
//
//   pio run   (platformio.ini: platform = atmelavr, board = megaatmega2560,
//              framework = arduino)
#include <Arduino.h>

void setup() {
  Serial.begin(115200);
  Serial1.begin(115200);
  Serial2.begin(115200);
  Serial.println("S0_BOOT");
  Serial1.println("S1_LINK_DATA");
  Serial2.println("S2_LINK_DATA");
  Serial.println("S0_DONE");
}

void loop() {}
