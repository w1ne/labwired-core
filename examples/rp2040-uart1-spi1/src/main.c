/* RP2040 UART1 + SPI1 + UART IRQ smoke. Register level, no SDK.
 *
 * 1. Release UART0, UART1 and SPI1 from RESETS and wait for RESET_DONE.
 * 2. UART1: 115200 8N1 at 125 MHz, print "UART1 OK".
 * 3. SPI1: PL022 loopback (SSPCR1.LBM), send 0xA5 and 0x3C, read both back.
 * 4. UART0: unmask TXIM; UART0_IRQ (NVIC 20) must be taken.
 * 5. UART1: unmask RXIM; a byte injected on UART1 RX must raise UART1_IRQ
 *    (NVIC 21); the handler echoes it on UART1.
 * 6. Print the verdict on UART0 and leave the results in SRAM.
 *
 * SRAM 0x20000100: [0] SPI1 byte 1  [1] SPI1 byte 2  [2] UART0 IRQs taken
 *                  [3] UART1 byte received  [4] 0xA55A when done  [5] 1 = pass
 */
#include <stdint.h>

#define REG(a) (*(volatile uint32_t *)(a))
#define RESETS_RESET   REG(0x4000C000u)
#define RESETS_DONE    REG(0x4000C008u)
#define RESET_SPI1     (1u << 17)
#define RESET_UART0    (1u << 22)
#define RESET_UART1    (1u << 23)

#define UART0 0x40034000u
#define UART1 0x40038000u
#define UDR(b)   REG((b) + 0x00u)
#define UFR(b)   REG((b) + 0x18u)
#define UIBRD(b) REG((b) + 0x24u)
#define UFBRD(b) REG((b) + 0x28u)
#define ULCRH(b) REG((b) + 0x2Cu)
#define UCR(b)   REG((b) + 0x30u)
#define UIMSC(b) REG((b) + 0x38u)
#define UMIS(b)  REG((b) + 0x40u)
#define UICR(b)  REG((b) + 0x44u)
#define FR_RXFE  (1u << 4)
#define FR_TXFF  (1u << 5)
#define INT_RX   (1u << 4)
#define INT_TX   (1u << 5)

#define SPI1 0x40040000u
#define SSPCR0  REG(SPI1 + 0x00u)
#define SSPCR1  REG(SPI1 + 0x04u)
#define SSPDR   REG(SPI1 + 0x08u)
#define SSPSR   REG(SPI1 + 0x0Cu)
#define SSPCPSR REG(SPI1 + 0x10u)
#define SR_RNE  (1u << 2)

#define NVIC_ISER REG(0xE000E100u)
#define NVIC_ICER REG(0xE000E180u)
#define UART0_IRQ 20u
#define UART1_IRQ 21u

#define RESULT ((volatile uint32_t *)0x20000100u)

void reset(void);
void default_handler(void) { for (;;) {} }
void uart0_handler(void);
void uart1_handler(void);

__attribute__((section(".vectors"), used))
const uintptr_t vectors[16 + 26] = {
    [0] = 0x20040000u,
    [1] = (uintptr_t)reset,
    [2] = (uintptr_t)default_handler,
    [3] = (uintptr_t)default_handler,
    [16 + UART0_IRQ] = (uintptr_t)uart0_handler,
    [16 + UART1_IRQ] = (uintptr_t)uart1_handler,
};

static void putc_on(uint32_t base, char c) {
    while (UFR(base) & FR_TXFF) {}
    UDR(base) = (uint8_t)c;
}

static void puts_on(uint32_t base, const char *s) {
    while (*s) putc_on(base, *s++);
}

static void uart_init(uint32_t base) {
    UIBRD(base) = 67u;          /* 125 MHz / (16 * 115200) = 67.81 */
    UFBRD(base) = 52u;
    ULCRH(base) = 0x70u;        /* 8 data bits, FIFO on */
    UCR(base) = 0x301u;         /* UARTEN | TXE | RXE */
}

void uart0_handler(void) {
    UIMSC(UART0) &= ~INT_TX;    /* one interrupt is the evidence */
    UICR(UART0) = INT_TX;
    RESULT[2] += 1;
}

void uart1_handler(void) {
    if (UMIS(UART1) & INT_RX) {
        while (!(UFR(UART1) & FR_RXFE)) {
            uint32_t b = UDR(UART1) & 0xFFu;
            RESULT[3] = b;
            putc_on(UART1, (char)b);
        }
        UICR(UART1) = INT_RX;
    }
}

static uint32_t spi_xfer(uint32_t v) {
    SSPDR = v;
    for (uint32_t n = 0; n < 100000u && !(SSPSR & SR_RNE); ++n) {}
    return SSPDR & 0xFFu;
}

int main(void);
void reset(void) { (void)main(); for (;;) {} }

int main(void) {
    for (uint32_t i = 0; i < 6u; ++i) RESULT[i] = 0;
    const uint32_t mask = RESET_SPI1 | RESET_UART0 | RESET_UART1;
    RESETS_RESET &= ~mask;
    for (uint32_t n = 0; n < 100000u && (RESETS_DONE & mask) != mask; ++n) {}

    uart_init(UART0);
    uart_init(UART1);
    puts_on(UART1, "UART1 OK\n");

    SSPCR0 = 0x07u;             /* 8-bit Motorola frames */
    SSPCPSR = 2u;
    SSPCR1 = 0x03u;             /* LBM | SSE */
    RESULT[0] = spi_xfer(0xA5u);
    RESULT[1] = spi_xfer(0x3Cu);

    NVIC_ISER = (1u << UART0_IRQ) | (1u << UART1_IRQ);
    UIMSC(UART0) = INT_TX;      /* TX FIFO is empty, so this asserts */
    for (volatile uint32_t n = 0; n < 1000u && RESULT[2] == 0; ++n) {}

    UIMSC(UART1) = INT_RX;
    for (volatile uint32_t n = 0; n < 2000000u && RESULT[3] == 0; ++n) {}

    uint32_t pass = RESULT[0] == 0xA5u && RESULT[1] == 0x3Cu &&
                    RESULT[2] == 1u && RESULT[3] == 'R';
    puts_on(UART0, pass ? "\nRP2040 UART1 SPI1 PASS\n" : "\nRP2040 UART1 SPI1 FAIL\n");
    RESULT[5] = pass;
    RESULT[4] = 0xA55Au;
    for (;;) {}
}
