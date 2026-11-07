#include <stdint.h>

#define REG(address) (*(volatile uint32_t *)(address))
#define GPIO_BASE 0x60004000u
#define GPIO_OUT1_W1TS REG(GPIO_BASE + 0x14u)
#define GPIO_OUT1_W1TC REG(GPIO_BASE + 0x18u)
#define GPIO_ENABLE1_W1TS REG(GPIO_BASE + 0x30u)

#define STATUS_LED_GPIO 35u
#define STATUS_LED_MASK (1u << (STATUS_LED_GPIO - 32u))

// Heltec maps the V4 ESP32-S3R2 white status LED to GPIO35. It is active-high.
__attribute__((noreturn, used, section(".entry")))
void _start(void) {
    GPIO_OUT1_W1TC = STATUS_LED_MASK;
    GPIO_ENABLE1_W1TS = STATUS_LED_MASK;

    for (;;) {
        GPIO_OUT1_W1TS = STATUS_LED_MASK;
        for (volatile uint32_t delay = 0; delay < 64u; ++delay) {
            __asm__ volatile("nop" ::: "memory");
        }
        GPIO_OUT1_W1TC = STATUS_LED_MASK;
        for (volatile uint32_t delay = 0; delay < 64u; ++delay) {
            __asm__ volatile("nop" ::: "memory");
        }
    }
}
