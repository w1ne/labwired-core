// Small AVR8 CPU/GPIO fixture for the imported ATmega32U4 profile.
// Build with avr-gcc -mmcu=atmega32u4 -DF_CPU=16000000UL -Os.
#include <avr/io.h>
#include <stdint.h>

int main(void) {
    DDRC = _BV(DDC7);

    for (;;) {
        PORTC = _BV(PORTC7);
        for (volatile uint16_t delay = 0; delay < 3000; ++delay) {
            __asm__ volatile("nop");
        }

        PORTC = 0;
        for (volatile uint16_t delay = 0; delay < 3000; ++delay) {
            __asm__ volatile("nop");
        }
    }
}
