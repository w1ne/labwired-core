// ATmega2560 16-bit timer fixture: Timer1/3/4/5 interrupts and PWM pads,
// written against DS2549 with avr-libc's vector names, so the handlers land at
// the Table 14-1 addresses the toolchain (not the simulator) chooses.
//
// Build (avr-gcc 7.3+, avr-libc 2.0+):
//   avr-gcc -mmcu=atmega2560 -Os -Wall -o tests/fixtures/avr/atmega2560-timers.elf
//       tests/fixtures/avr/atmega2560-timers.c
//
// 16 MHz clock. Configuration (DS2549 section 17, Table 17-2):
//   Timer1  WGM 4  CTC, TOP = OCR1A = 999, clk/8   -> COMPA 2 kHz, OCR1C = 500
//           gives COMPC at 2 kHz as well (the stepper-style tick).
//   Timer3  WGM 5  fast PWM 8-bit, clk/64          -> TOV3 at TOP, 976.5625 Hz;
//           OC3A/B/C (PE3/PE4/PE5) at OCR 64/128/192, non-inverting.
//   Timer4  WGM 14 fast PWM, TOP = ICR4 = 1999, clk/8 -> 1 kHz; ICF4 is set at
//           TOP (17.11.39); OC4A/B/C (PH3/PH4/PH5) at OCR 499/999/1499.
//   Timer5  WGM 4  CTC, TOP = OCR5A = 249, clk/64  -> COMPA 1 kHz.
// After 100 Timer5 ticks (100 ms) interrupts go off and the counts are printed
// once on USART0 as "T1A=.. T1C=.. T3OVF=.. T4CAPT=.. T5A=..\n"; the PWM keeps
// running.
#include <avr/interrupt.h>
#include <avr/io.h>
#include <stdint.h>

static volatile uint16_t n1a, n1c, n3ovf, n4capt, n5a;

ISR(TIMER1_COMPA_vect) { n1a++; }
ISR(TIMER1_COMPC_vect) { n1c++; }
ISR(TIMER3_OVF_vect) { n3ovf++; }
ISR(TIMER4_CAPT_vect) { n4capt++; }
ISR(TIMER5_COMPA_vect) { n5a++; }

static void put(char c) {
    while (!(UCSR0A & _BV(UDRE0))) {
    }
    UDR0 = c;
}

static void puts_(const char *s) {
    while (*s) {
        put(*s++);
    }
}

static void put_u16(uint16_t v) {
    char buf[6];
    uint8_t i = 0;
    do {
        buf[i++] = '0' + v % 10;
        v /= 10;
    } while (v);
    while (i) {
        put(buf[--i]);
    }
}

static void field(const char *name, uint16_t v) {
    puts_(name);
    put('=');
    put_u16(v);
}

int main(void) {
    UBRR0 = 8; // 115200 baud at 16 MHz (U2X0 = 0)
    UCSR0B = _BV(TXEN0);

    DDRE |= _BV(PE3) | _BV(PE4) | _BV(PE5);
    DDRH |= _BV(PH3) | _BV(PH4) | _BV(PH5);

    OCR1A = 999;
    OCR1C = 500;
    TCCR1A = 0;
    TIMSK1 = _BV(OCIE1A) | _BV(OCIE1C);

    OCR3A = 64;
    OCR3B = 128;
    OCR3C = 192;
    TCCR3A = _BV(COM3A1) | _BV(COM3B1) | _BV(COM3C1) | _BV(WGM30);
    TIMSK3 = _BV(TOIE3);

    ICR4 = 1999;
    OCR4A = 499;
    OCR4B = 999;
    OCR4C = 1499;
    TCCR4A = _BV(COM4A1) | _BV(COM4B1) | _BV(COM4C1) | _BV(WGM41);
    TIMSK4 = _BV(ICIE4);

    OCR5A = 249;
    TCCR5A = 0;
    TIMSK5 = _BV(OCIE5A);

    // Start all four together.
    TCCR1B = _BV(WGM12) | _BV(CS11);
    TCCR3B = _BV(WGM32) | _BV(CS31) | _BV(CS30);
    TCCR4B = _BV(WGM43) | _BV(WGM42) | _BV(CS41);
    TCCR5B = _BV(WGM52) | _BV(CS51) | _BV(CS50);
    sei();

    while (n5a < 100) {
    }
    cli();
    field("T1A", n1a);
    put(' ');
    field("T1C", n1c);
    put(' ');
    field("T3OVF", n3ovf);
    put(' ');
    field("T4CAPT", n4capt);
    put(' ');
    field("T5A", n5a);
    put('\n');
    for (;;) {
    }
}
