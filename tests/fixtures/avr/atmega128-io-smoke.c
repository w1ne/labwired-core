// GPIOA + polled USART0 smoke for the imported ATmega128 profile. The
// ATmega128 keeps the classic I/O file (USART0 at UBRR0L 0x29..UDR0 0x2C,
// PINA/DDRA/PORTA at 0x39..0x3B, read-only PINx), adds extended I/O at
// 0x60..0xFF, and starts SRAM at 0x100.
// Build: avr-gcc -mmcu=atmega128 -Os -o atmega128-io-smoke.elf atmega128-io-smoke.c
#include <avr/io.h>
#include <stdint.h>

volatile uint8_t result_bits;
// Lands at 0x122..0x1C1, after .data (SRAM from 0x100).
volatile uint8_t scratch[160];

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

    UBRR0L = 51;
    UCSR0B = _BV(TXEN0);
    if (UBRR0L == 51) bits |= 0x01;
    if (UCSR0B == _BV(TXEN0)) bits |= 0x02;
    if (UCSR0A & _BV(UDRE0)) bits |= 0x04;

    DDRA = pa0;
    PORTA = _BV(PA7);
    PORTA |= pa0; // SBI: read-modify-write keeps PA7 (0x3B is not PCIFR).
    if (PORTA == (pa0 | _BV(PA7)) && (PINA & pa0)) bits |= 0x08;
    PORTA &= (uint8_t)~_BV(PA7); // CBI clears PA7 only.
    if (PORTA == pa0) bits |= 0x10;
    PINA = pa0; // Read-only on ATmega128: no toggle.
    if ((PINA & pa0) && PORTA == pa0) bits |= 0x20;

    for (uint8_t i = 0; i < sizeof scratch; ++i) {
        scratch[i] = (uint8_t)(i ^ 0xA5);
    }
    uint8_t ok = 1;
    for (uint8_t i = 0; i < sizeof scratch; ++i) {
        if (scratch[i] != (uint8_t)(i ^ 0xA5)) ok = 0;
    }
    if (ok) bits |= 0x40;

    result_bits = bits;
    puts_("M128 IO bits=");
    hex8(bits);
    puts_("\r\n");

    for (;;) {
        PORTA ^= pa0;
        for (volatile uint16_t i = 0; i < 200; ++i) {
        }
    }
}
