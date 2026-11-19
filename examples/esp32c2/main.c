/* ESP32-C2 (ESP8684H4X) bare-metal smoke for the LabWired simulator.
 *
 * 1. Checks a MUL/DIVU result (RV32IMC M extension) computed from volatile
 *    operands, so GCC cannot fold it.
 * 2. Writes and reads back the last word of the 256 KiB DRAM view
 *    (0x3FCDFFFC), the top edge of the descriptor's `ram` region.
 * 3. Prints one line on UART0 (FIFO at 0x60000000) only if both pass.
 * 4. Toggles GPIO10 through GPIO_ENABLE / GPIO_OUT forever.
 */
#include <stdint.h>

#define REG(a) (*(volatile uint32_t *)(a))
#define UART0_FIFO 0x60000000u
#define UART0_STATUS 0x6000001Cu
#define GPIO_OUT 0x60004004u
#define GPIO_ENABLE 0x60004020u
#define DRAM_LAST_WORD 0x3FCDFFFCu
#define PIN 10u

static void putc0(char c) {
    while (((REG(UART0_STATUS) >> 16) & 0x3FFu) >= 126u) {
    }
    REG(UART0_FIFO) = (uint8_t)c;
}

static void puts0(const char *s) {
    while (*s) {
        putc0(*s++);
    }
}

int main(void) {
    volatile uint32_t a = 1234567u, b = 89u;
    uint32_t product = a * b;
    uint32_t quotient = product / b;
    REG(DRAM_LAST_WORD) = 0xC2C2A55Au;
    if (product == 109876463u && quotient == 1234567u && REG(DRAM_LAST_WORD) == 0xC2C2A55Au) {
        puts0("Hello from ESP32-C2\r\n");
    } else {
        puts0("FAIL\r\n");
    }
    REG(GPIO_ENABLE) |= 1u << PIN;
    for (;;) {
        REG(GPIO_OUT) ^= 1u << PIN;
        for (volatile uint32_t i = 0; i < 200u; i++) {
        }
    }
}
