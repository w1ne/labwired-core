// GPIOB + polled USART smoke for the imported ATmega8 profile. The ATmega8
// has the classic 64-byte I/O file but no port A: USART at UBRRL 0x29..UDR
// 0x2C, PINB/DDRB/PORTB at 0x36..0x38, read-only PINx, and SRAM from 0x60.
// Build: avr-gcc -mmcu=atmega8 -Os -o atmega8-io-smoke.elf atmega8-io-smoke.c
#include <avr/io.h>
#include <stdint.h>

volatile uint8_t result_bits;
// Lands at 0x80..0x11F (avr-gcc 0x60 SRAM start): SRAM that the 328P map
// would decode as timers, ADC, TWI and USART0.
volatile uint8_t scratch[160];

static void tx(uint8_t c) {
    while (!(UCSRA & _BV(UDRE))) {
    }
    UDR = c;
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
    const uint8_t pb0 = _BV(PB0);

    UBRRL = 51;
    UCSRB = _BV(TXEN);
    if (UBRRL == 51) bits |= 0x01;
    if (UCSRB == _BV(TXEN)) bits |= 0x02;
    if (UCSRA & _BV(UDRE)) bits |= 0x04;

    DDRB = pb0;
    PORTB = _BV(PB7);
    PORTB |= pb0; // SBI: read-modify-write keeps PB7.
    if (PORTB == (pb0 | _BV(PB7)) && (PINB & pb0)) bits |= 0x08;
    PORTB &= (uint8_t)~_BV(PB7); // CBI clears PB7 only.
    if (PORTB == pb0) bits |= 0x10;
    PINB = pb0; // Read-only on ATmega8: no toggle.
    if ((PINB & pb0) && PORTB == pb0) bits |= 0x20;

    for (uint8_t i = 0; i < sizeof scratch; ++i) {
        scratch[i] = (uint8_t)(i ^ 0xA5);
    }
    uint8_t ok = 1;
    for (uint8_t i = 0; i < sizeof scratch; ++i) {
        if (scratch[i] != (uint8_t)(i ^ 0xA5)) ok = 0;
    }
    if (ok) bits |= 0x40;

    result_bits = bits;
    puts_("M8 IO bits=");
    hex8(bits);
    puts_("\r\n");

    for (;;) {
        PORTB ^= pb0;
        for (volatile uint16_t i = 0; i < 200; ++i) {
        }
    }
}
