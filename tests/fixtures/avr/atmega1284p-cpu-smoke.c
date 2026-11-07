// CPU and SRAM smoke for the imported ATmega1284P profile.
// Build with avr-gcc -mmcu=atmega1284p -Os.
#include <stdint.h>

volatile uint16_t progress;

int main(void) {
    progress = 0;
    for (;;) {
        ++progress;
        __asm__ volatile("nop");
    }
}
