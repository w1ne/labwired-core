// Arduino Uno sketch for the Timer2 tone() twin.
//
// tone() is non-blocking. Each delay() covers that call's duration plus a
// gap, so the two bursts on D6 are separate and the pin is idle afterwards.
// Built with ArduinoCore-avr 1.8.6 (the TIMER2_COMPA ISR calls noTone(),
// which is what lets the second tone() program CTC mode again):
//
//   avr-gcc -mmcu=atmega328p -DF_CPU=16000000L -DARDUINO=10806 \
//     -DARDUINO_AVR_UNO -DARDUINO_ARCH_AVR -Os -ffunction-sections \
//     -fdata-sections -fno-exceptions -I<core> -I<variants/standard> \
//     -c <core>/{hooks,wiring,wiring_digital,wiring_analog}.c Tone.cpp \
//        main.cpp new.cpp abi.cpp WInterrupts.c this file
//   avr-g++ -mmcu=atmega328p -Os -Wl,--gc-sections -o arduino-uno-tone.elf *.o
#include <Arduino.h>

void setup() {
  tone(6, 700, 80);
  delay(120);
  tone(6, 700, 240);
  delay(280);
}

void loop() {}
