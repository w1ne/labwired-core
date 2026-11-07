// GPIO smoke for the imported ATtiny13, 24/44/84 and 25/45/85 profiles. These
// parts have no USART, so the result is a byte in SRAM plus PB0 edges.
// Classic 64-byte I/O file: PINB/DDRB/PORTB at 0x36..0x38 (x4 adds
// PINA/DDRA/PORTA at 0x39..0x3B), write-one-to-toggle PINx, SRAM from 0x60.
// Build with scripts/build_attiny_portb_io_smoke.sh.
#include <avr/io.h>
#include <stdint.h>

#ifndef SCRATCH_LEN
#define SCRATCH_LEN 48
#endif

volatile uint8_t result_bits;
volatile uint8_t scratch[SCRATCH_LEN];

int main(void) {
    uint8_t bits = 0;
    const uint8_t pb0 = _BV(PB0);

    DDRB = pb0 | _BV(PB3);
    if (DDRB == (pb0 | _BV(PB3))) bits |= 0x01;
    PORTB = _BV(PB3);
    PORTB |= pb0; // SBI: read-modify-write keeps PB3.
    if (PORTB == (pb0 | _BV(PB3)) && (PINB & pb0)) bits |= 0x02;
    PORTB &= (uint8_t)~_BV(PB3); // CBI clears PB3 only.
    if (PORTB == pb0) bits |= 0x04;
    PINB = pb0; // Write-one-to-toggle: PB0 goes low.
    if (!(PINB & pb0) && PORTB == 0) bits |= 0x08;
    PORTB = _BV(PB3);
    __asm__ volatile("sbi %0, 0" ::"I"(_SFR_IO_ADDR(PINB))); // toggles PB0 only
    if (PORTB == (pb0 | _BV(PB3))) bits |= 0x10;
    PORTB = 0;

    for (uint8_t i = 0; i < sizeof scratch; ++i) {
        scratch[i] = (uint8_t)(i ^ 0xA5);
    }
    uint8_t ok = 1;
    for (uint8_t i = 0; i < sizeof scratch; ++i) {
        if (scratch[i] != (uint8_t)(i ^ 0xA5)) ok = 0;
    }
    if (ok) bits |= 0x20;

#ifdef PORTA
    DDRA = _BV(PA7);
    PORTA = _BV(PA7);
    PORTA |= _BV(PA1);
    if (PORTA == (_BV(PA7) | _BV(PA1)) && (PINA & _BV(PA7))) bits |= 0x40;
    PINA = _BV(PA7);
    if (PORTA != _BV(PA1)) bits &= (uint8_t)~0x40;
#endif

    result_bits = bits;

    for (;;) {
        PINB = pb0;
        for (volatile uint8_t i = 0; i < 200; ++i) {
        }
    }
}
