#include <stdint.h>

#define REG(address) (*(volatile uint32_t *)(address))
#define GPIO_BASE 0x60091000u
#define GPIO_OUT_W1TS REG(GPIO_BASE + 0x08u)
#define GPIO_OUT_W1TC REG(GPIO_BASE + 0x0Cu)
#define GPIO_ENABLE_W1TS REG(GPIO_BASE + 0x24u)

#define USER_LED_GPIO 15u
#define USER_LED_MASK (1u << USER_LED_GPIO)

// Seeed's XIAO ESP32-C6 schematic wires the yellow USER LED to GPIO15. The
// LED/resistor path runs from 3V3 into the GPIO, so it is active-low.
__attribute__((noreturn))
void main(void) {
    GPIO_OUT_W1TS = USER_LED_MASK; /* off */
    GPIO_ENABLE_W1TS = USER_LED_MASK;

    for (;;) {
        GPIO_OUT_W1TC = USER_LED_MASK; /* on */
        for (volatile uint32_t delay = 0; delay < 64u; ++delay) {
            __asm__ volatile("nop" ::: "memory");
        }
        GPIO_OUT_W1TS = USER_LED_MASK; /* off */
        for (volatile uint32_t delay = 0; delay < 64u; ++delay) {
            __asm__ volatile("nop" ::: "memory");
        }
    }
}
