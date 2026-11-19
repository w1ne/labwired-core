// GPIOA + polled USART0 smoke for the imported ATmega164/324/644/1284P and
// ATmega1280/1281 profiles (port A at 0x20 and USART0 at 0xC0 on all of them).
// Build: avr-gcc -mmcu=atmega164p -Os -o atmega164-io-smoke.elf mega164-io-smoke.c
//        avr-gcc -mmcu=atmega324p -Os -DPART_TAG='"324"' -o atmega324-io-smoke.elf mega164-io-smoke.c
//        avr-gcc -mmcu=atmega644 -Os -DPART_TAG='"644"' -o atmega644-io-smoke.elf mega164-io-smoke.c
//        avr-gcc -mmcu=atmega1284p -Os -DPART_TAG='"1284P"' -o atmega1284p-io-smoke.elf mega164-io-smoke.c
//        avr-gcc -mmcu=atmega1280 -Os -DPART_TAG='"1280"' -o atmega1280-io-smoke.elf mega164-io-smoke.c
//        avr-gcc -mmcu=atmega1281 -Os -DPART_TAG='"1281"' -o atmega1281-io-smoke.elf mega164-io-smoke.c
#include <avr/io.h>
#include <stdint.h>

#ifndef PART_TAG
#define PART_TAG "164"
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
    const uint8_t pa0 = _BV(PA0);

    // Polled USART0 TX; the fixture is the profile's only serial observer.
    UBRR0H = 0;
    UBRR0L = 103;
    UCSR0C = _BV(UCSZ01) | _BV(UCSZ00);
    UCSR0B = _BV(TXEN0);
    if (UBRR0L == 103) bits |= 0x01;
    if (UCSR0B == _BV(TXEN0)) bits |= 0x02;
    if (UCSR0A & _BV(UDRE0)) bits |= 0x04;

    DDRA = pa0;
    PORTA = pa0;
    if (DDRA == pa0) bits |= 0x08;
    if (PINA & pa0) bits |= 0x10;
    PINA = pa0; // PINx write-one-to-toggle: PA0 low.
    if (!(PINA & pa0) && !(PORTA & pa0)) bits |= 0x20;
    PINA = pa0; // And high again, proving the toggle can be repeated.
    if ((PINA & pa0) && (PORTA & pa0)) bits |= 0x40;

    result_bits = bits;
    puts_("M" PART_TAG " IO bits=");
    hex8(bits);
    puts_("\r\n");

    for (;;) {
        PORTA ^= pa0;
        for (volatile uint16_t i = 0; i < 200; ++i) {
        }
    }
}
