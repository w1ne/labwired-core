// GPIOA + polled USART smoke for the imported ATmega16 profile. ATmega16 has
// the classic 64-byte I/O file: USART at UBRRL 0x29..UDR 0x2C, PINA/DDRA/PORTA
// at 0x39..0x3B, read-only PINx, and SRAM from 0x60.
// Build: avr-gcc -mmcu=atmega16 -Os -o atmega16-io-smoke.elf atmega16-io-smoke.c
#include <avr/io.h>
#include <stdint.h>

volatile uint8_t result_bits;
// Lands at 0x82..0x121 (avr-gcc 0x60 SRAM start): SRAM that the 328P map
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
    const uint8_t pa0 = _BV(PA0);

    UBRRL = 51;
    UCSRB = _BV(TXEN);
    if (UBRRL == 51) bits |= 0x01;
    if (UCSRB == _BV(TXEN)) bits |= 0x02;
    if (UCSRA & _BV(UDRE)) bits |= 0x04;

    DDRA = pa0;
    PORTA = _BV(PA7);
    PORTA |= pa0; // SBI: read-modify-write keeps PA7.
    if (PORTA == (pa0 | _BV(PA7)) && (PINA & pa0)) bits |= 0x08;
    PORTA &= (uint8_t)~_BV(PA7); // CBI clears PA7 only.
    if (PORTA == pa0) bits |= 0x10;
    PINA = pa0; // Read-only on ATmega16: no toggle.
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
    puts_("M16 IO bits=");
    hex8(bits);
    puts_("\r\n");

    for (;;) {
        PORTA ^= pa0;
        for (volatile uint16_t i = 0; i < 200; ++i) {
        }
    }
}
