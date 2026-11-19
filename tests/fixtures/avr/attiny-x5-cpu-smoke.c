// CPU and SRAM smoke for the imported classic tinyAVR profiles
// (ATtiny13, 2313/2313A/4313, 24/44/84, 25/45/85, 1634) and the classic
// megaAVR profiles (ATmega8, 48, 88, 32, 164, 324, 644, 169P, 16M1, 64M1,
// 128RFA1, 128RFR2) and the AT90USB162.
// Build with scripts/build_attiny_x5_cpu_smoke.sh and
// scripts/build_classic_mega_cpu_smoke.sh.
#include <stdint.h>

volatile uint16_t progress;

int main(void) {
    progress = 0;
    for (;;) {
        ++progress;
        __asm__ volatile("nop");
    }
}
