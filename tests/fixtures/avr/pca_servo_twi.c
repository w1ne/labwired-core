/* Minimal ATmega328P TWI fixture, compiled with avr-gcc -mmcu=atmega328p
 * -Os -nostdlib -Wl,-e,main. Exercises modeled TWCR/TWDR transactions. */
#include <stdint.h>
#define TWDR (*(volatile uint8_t *)0xbb)
#define TWCR (*(volatile uint8_t *)0xbc)
static void transaction(uint8_t reg, const uint8_t *data, uint8_t count) {
    TWCR = 0xa4;
    TWDR = 0x82; TWCR = 0x84;
    TWDR = reg; TWCR = 0x84;
    while (count--) { TWDR = *data++; TWCR = 0x84; }
    TWCR = 0x94;
}
void main(void) {
    const uint8_t sleep[] = {0x10}, prescale[] = {121}, wake[] = {0x20};
    const uint8_t pulse[] = {0, 0, 0x33, 1}, off[] = {0, 0, 0, 0x10};
    transaction(0, sleep, 1); transaction(0xfe, prescale, 1); transaction(0, wake, 1);
    transaction(6, pulse, 4); transaction(6, off, 4);
    for (;;) {}
}
