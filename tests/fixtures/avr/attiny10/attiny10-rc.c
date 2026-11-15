// ATtiny10 (AVRrc) firmware fixture for crates/core/tests/avr/avr_attiny10.rs.
// Build: avr-gcc -mmcu=attiny10 -Os -o attiny10-rc.elf attiny10-rc.c
// Exercises the reduced core's flash-mapped constants (LD from 0x4000+),
// .data initialisation, RCALL/RET on the 32-byte stack and PORTB.
#include <avr/io.h>

static const unsigned char table[4] = {0x11, 0x22, 0x33, 0x44};
volatile unsigned char counter = 5;
volatile unsigned char sum;

__attribute__((noinline)) static unsigned char fetch(const unsigned char *p, unsigned char i)
{
    return p[i];
}

__attribute__((noinline)) static unsigned char add(unsigned char a, unsigned char b)
{
    return a + b;
}

int main(void)
{
    DDRB = 0x0F;
    unsigned char s = 0;
    for (unsigned char i = 0; i < 4; i++) {
        s = add(s, fetch(table, i));
    }
    sum = s;                          // 0xAA
    counter++;                        // 6
    PORTB = (s ^ counter) & 0x0F;     // 0x0C
    for (;;) {
    }
}
