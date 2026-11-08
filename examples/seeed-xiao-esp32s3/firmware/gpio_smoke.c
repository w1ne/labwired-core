#include <stdint.h>

#define REG(address) (*(volatile uint32_t *)(address))
#define GPIO_BASE 0x60004000u
#define GPIO_OUT_W1TS REG(GPIO_BASE + 0x08u)
#define GPIO_OUT_W1TC REG(GPIO_BASE + 0x0Cu)
#define GPIO_ENABLE_W1TS REG(GPIO_BASE + 0x24u)

#define USER_LED_GPIO 21u
#define USER_LED_MASK (1u << USER_LED_GPIO)

// Seeed's standard XIAO ESP32-S3 routes the active-low user LED to GPIO21.
__attribute__((noreturn, used, section(".entry")))
void _start(void) {
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
