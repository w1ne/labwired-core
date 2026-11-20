// GPIOB + polled USART smoke for the imported ATtiny2313/2313A/4313 profiles.
// Classic 64-byte I/O file: USART at UBRRL 0x29..UDR 0x2C, PINB/DDRB/PORTB at
// 0x36..0x38, write-one-to-toggle PINx, and SRAM from 0x60.
// Build: avr-gcc -mmcu=attiny2313 -Os -DPART_TAG='"2313"' \
//            -o attiny2313-io-smoke.elf attiny2313-io-smoke.c
#include <avr/io.h>
#include <stdint.h>

#ifndef PART_TAG
#define PART_TAG "2313"
#endif

volatile uint8_t result_bits;
// 48 bytes at 0x84..0xB3 (after .data): SRAM the 328P map would decode as
// its extended I/O (timers, ADC, USART0).
// Kept small so the 128-byte part still has room for the stack.
volatile uint8_t scratch[48];

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
    PINB = pb0; // Write-one-to-toggle: PB0 goes low.
    if (!(PINB & pb0) && PORTB == 0) bits |= 0x20;

    for (uint8_t i = 0; i < sizeof scratch; ++i) {
        scratch[i] = (uint8_t)(i ^ 0xA5);
    }
    uint8_t ok = 1;
    for (uint8_t i = 0; i < sizeof scratch; ++i) {
        if (scratch[i] != (uint8_t)(i ^ 0xA5)) ok = 0;
    }
    if (ok) bits |= 0x40;

    result_bits = bits;
    puts_("T" PART_TAG " IO bits=");
    hex8(bits);
    puts_("\r\n");

    for (;;) {
        PINB = pb0;
        for (volatile uint16_t i = 0; i < 200; ++i) {
        }
    }
}
