// CPU and SRAM smoke for the imported ATmega1280 profile.
// Build with avr-gcc -mmcu=atmega1280 -Os.
#include <stdint.h>

volatile uint16_t progress;

int main(void) {
    progress = 0;
    for (;;) {
        ++progress;
        __asm__ volatile("nop");
    }
}
