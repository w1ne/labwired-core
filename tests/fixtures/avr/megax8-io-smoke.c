// USART0 + PORTB/PORTD smoke for the imported megaAVR x8 profiles
// (ATmega48/88/168). All three share the ATmega328P register summary:
// PINB/DDRB/PORTB at 0x23..0x25, PIND/DDRD/PORTD at 0x29..0x2B, USART0 at
// 0xC0..0xC6. Build once per part:
//   avr-gcc -mmcu=atmega168 -DPART_TAG='"M168"' -Os -o atmega168-io-smoke.elf megax8-io-smoke.c
#include <avr/io.h>
#include <stdint.h>

#ifndef PART_TAG
#define PART_TAG "MX8"
#endif

volatile uint8_t result_bits;

static void tx(uint8_t c) {
    while (!(UCSR0A & _BV(UDRE0))) {
    }
    UDR0 = c;
}

static void puts_(const char *s) {
    while (*s) {
        tx((uint8_t)*s++);
    }
}

static void hex8(uint8_t v) {
    static const char digits[] = "0123456789ABCDEF";
    tx(digits[v >> 4]);
    tx(digits[v & 0x0F]);
}

int main(void) {
    uint8_t bits = 0;

    // USART0 8N1 at 9600 baud for 16 MHz: control registers read back.
    UBRR0H = 0;
    UBRR0L = 103;
    UCSR0C = _BV(UCSZ01) | _BV(UCSZ00);
    UCSR0B = _BV(TXEN0);
    if (UBRR0L == 103) bits |= 0x01;
    if (UCSR0B == _BV(TXEN0)) bits |= 0x02;
    if (UCSR0A & _BV(UDRE0)) bits |= 0x04;

    // PB5 output: DDRB reads back and PINB shows the driven latch.
    DDRB = _BV(DDB5);
    PORTB = _BV(PORTB5);
    if (DDRB == _BV(DDB5)) bits |= 0x08;
    if (PINB & _BV(PINB5)) bits |= 0x10;
    PORTB = 0;
    if (!(PINB & _BV(PINB5))) bits |= 0x20;

    // PD2 output on the second port, independent of PORTB.
    DDRD = _BV(DDD2);
    PORTD = _BV(PORTD2);
    if ((PIND & _BV(PIND2)) && !(PINB & _BV(PINB5))) bits |= 0x40;

    result_bits = bits;
    puts_(PART_TAG " IO bits=");
    hex8(bits);
    puts_("\r\n");

    for (;;) {
        PORTB ^= _BV(PORTB5);
        for (volatile uint16_t i = 0; i < 200; ++i) {
        }
    }
}
