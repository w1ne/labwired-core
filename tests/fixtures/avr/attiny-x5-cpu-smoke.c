// CPU and SRAM smoke for the imported ATtiny25/45/85 profiles.
// Build with scripts/build_attiny_x5_cpu_smoke.sh.
#include <stdint.h>

volatile uint16_t progress;

int main(void) {
    progress = 0;
    for (;;) {
        ++progress;
        __asm__ volatile("nop");
    }
}
