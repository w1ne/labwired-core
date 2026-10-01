/*
 * F103 images. One source, seven ELFs. Each image clocks the peripherals it
 * uses, stays inside the 20 KB SRAM, and prints its marker.
 *
 *   control                         BENCH_UART_OK
 *   clockbug  (-DSKIP_UART_CLOCK)   BENCH_UART_OK   TXE stays clear while USART1 is gated
 *   gpiobug   (-DGPIO_CLOCK_BUG)    BENCH_GPIO_OK   GPIOA ODR does not read back while gated
 *   rambug    (-DRAM_OVERFLOW)      BENCH_RAM_OK    store past 20 KB takes the fault handler
 *   irqtime   (-DIRQ_TIME)          BENCH_UIF_OK    TIM2 UIF still clear
 *   nvicclear (-DNVIC_CLEAR)        BENCH_NVIC_OK   cleared pending does not run
 *   usartmux  (-DUSART_MUX_BUG)     BENCH_UART_OK   poison while PA9 is GPIO must stay off the pad
 */

#include <stdint.h>

#define REG32(addr) (*(volatile uint32_t *) (addr))

/* --- RCC (F1): peripheral clock enables (RM0008 §7.3.7) --- */
#define RCC_BASE 0x40021000u
#define RCC_APB2ENR REG32(RCC_BASE + 0x18u)
#define RCC_APB1ENR REG32(RCC_BASE + 0x1Cu)
#define RCC_APB2ENR_USART1EN (1u << 14)
#define RCC_APB2ENR_IOPAEN (1u << 2)
#define RCC_APB1ENR_TIM2EN (1u << 0)

/* TIM2, general-purpose, 16-bit (RM0008 §15). */
#define TIM2_BASE 0x40000000u
#define TIM2_CR1 REG32(TIM2_BASE + 0x00u)
#define TIM2_DIER REG32(TIM2_BASE + 0x0Cu)
#define TIM2_SR REG32(TIM2_BASE + 0x10u)
#define TIM2_PSC REG32(TIM2_BASE + 0x28u)
#define TIM2_ARR REG32(TIM2_BASE + 0x2Cu)

/* NVIC IRQ0 (exception 16, WWDG). */
#define NVIC_ISER0 REG32(0xE000E100u)
#define NVIC_ISPR0 REG32(0xE000E200u)
#define NVIC_ICPR0 REG32(0xE000E280u)

/* --- GPIOA (F1 layout: CRL @ 0x00, CRH @ 0x04, ODR @ 0x0C). The F1 pad mux is
 * four bits per pin — MODE[1:0] then CNF[1:0]. There is no MODER and no AFR on
 * this family, so there is no AF number to write. --- */
#define GPIOA_BASE 0x40010800u
#define GPIOA_CRL REG32(GPIOA_BASE + 0x00u)
#define GPIOA_CRH REG32(GPIOA_BASE + 0x04u)
#define GPIOA_ODR REG32(GPIOA_BASE + 0x0Cu)
/* PA9 carries USART1_TX in the **Default** alternate-function column
 * (DS5319 Rev 20, Table 5, p.31), so no AFIO remap is involved. Its CRH nibble
 * is bits [7:4]; 0xB is MODE 0b11 (output, 50 MHz) + CNF 0b10 (alternate
 * function, push-pull). */
#define GPIOA_CRH_PA9_SHIFT 4u
#define CRH_AF_PUSH_PULL_50MHZ 0xBu
/* MODE 0b11, CNF 0b00: GPIO push-pull, 50 MHz. Not an alternate function. */
#define CRH_GPIO_PUSH_PULL_50MHZ 0x3u

/* --- USART1 (F1 layout: SR @ 0x00, DR @ 0x04, BRR @ 0x08, CR1 @ 0x0C) --- */
#define USART1_BASE 0x40013800u
#define U1_SR REG32(USART1_BASE + 0x00u)
#define U1_DR REG32(USART1_BASE + 0x04u)
#define U1_BRR REG32(USART1_BASE + 0x08u)
#define U1_CR1 REG32(USART1_BASE + 0x0Cu)
#define SR_TXE (1u << 7)
#define CR1_UE (1u << 13)
#define CR1_TE (1u << 3)

/* BRR = f_PCLK2 / baud at the default 16x oversampling. This firmware never
 * touches the PLL, so the part runs on the 8 MHz HSI it selects at reset
 * (DS5319 Rev 20 section 2.3.7, p.15): 8000000 / 115200 = 69.44 -> 69 = 0x45. */
#define U1_BRR_115200_AT_8MHZ 69u

/* Mux PA9 and program the divisor, then enable the transmitter. */
static void uart_puts(const char *s);

static volatile uint32_t irq_ran;

static void uart_init(void)
{
    GPIOA_CRH = (GPIOA_CRH & ~(0xFu << GPIOA_CRH_PA9_SHIFT))
                | (CRH_AF_PUSH_PULL_50MHZ << GPIOA_CRH_PA9_SHIFT);
    U1_BRR = U1_BRR_115200_AT_8MHZ;
    U1_CR1 = CR1_UE | CR1_TE;
}

/* IRQ0 handler. Linked from the vector table. */
void Bench_IRQ0(void)
{
    irq_ran = 1u;
}

static void uart_putc(char c)
{
    while ((U1_SR & SR_TXE) == 0u) {
    }
    U1_DR = (uint32_t) (uint8_t) c;
}

static void uart_puts(const char *s)
{
    while (*s) uart_putc(*s++);
}

/* HardFault from the rambug store. Other images just stop here. */
void Bench_HardFault(void)
{
#ifdef RAM_OVERFLOW
    uart_puts("BENCH_RAM_OK\n");
#endif
    for (;;) {
    }
}

int main(void)
{
    RCC_APB2ENR |= RCC_APB2ENR_USART1EN;
    RCC_APB2ENR |= RCC_APB2ENR_IOPAEN;
    uart_init();

#ifdef RAM_OVERFLOW
    /* 0x2000_6000 is 4 KB past the F103C8's 20 KB SRAM. A real map faults
     * before the banner. An oversized map stores the word and falls through. */
    volatile uint32_t *oob = (volatile uint32_t *) 0x20006000u;
    *oob = 0xCAFEBABEu;
    uart_puts("BENCH_BANNER\n");
#elif defined(GPIO_CLOCK_BUG)
    /* Pad is already muxed, so the report path survives gating GPIOA. */
    uart_puts("BENCH_BANNER\n");
    RCC_APB2ENR &= ~RCC_APB2ENR_IOPAEN;
    GPIOA_CRL = 0x33333333u;
    GPIOA_ODR = 0x000000FFu;
    if ((GPIOA_ODR & 0x000000FFu) != 0x000000FFu) {
        uart_puts("BENCH_GPIO_OK\n");
    }
#elif defined(SKIP_UART_CLOCK)
    uart_puts("BENCH_BANNER\n");
    RCC_APB2ENR &= ~RCC_APB2ENR_USART1EN;
    int txe_seen = 0;
    for (uint32_t i = 0; i < 64u; i++) {
        if ((U1_SR & SR_TXE) != 0u) {
            txe_seen = 1;
            break;
        }
    }
    RCC_APB2ENR |= RCC_APB2ENR_USART1EN;
    uart_init();
    if (txe_seen == 0) {
        uart_puts("BENCH_UART_OK\n");
    }
#elif defined(IRQ_TIME)
    /* Update event must still be clear a few cycles after CEN. */
    RCC_APB1ENR |= RCC_APB1ENR_TIM2EN;
    TIM2_ARR = 1000u;
    TIM2_PSC = 0u;
    TIM2_DIER = 1u;
    TIM2_CR1 = 1u;
    for (volatile uint32_t i = 0; i < 20u; i++) {
    }
    uart_puts("BENCH_BANNER\n");
    if ((TIM2_SR & 1u) == 0u) {
        uart_puts("BENCH_UIF_OK\n");
    }
#elif defined(NVIC_CLEAR)
    uart_puts("BENCH_BANNER\n");
    __asm volatile("cpsid i" ::: "memory");
    NVIC_ISER0 = 1u;
    NVIC_ISPR0 = 1u;
    NVIC_ICPR0 = 1u;
    __asm volatile("cpsie i\n\tdsb\n\tisb" ::: "memory");
    for (volatile uint32_t i = 0; i < 50u; i++) {
    }
    if (irq_ran == 0u) {
        uart_puts("BENCH_NVIC_OK\n");
    }
#elif defined(USART_MUX_BUG)
    /* PA9 leaves USART1_TX. DR still takes the poison byte; the pad must not.
     * The mux is restored before the marker, so the report path is a real TX. */
    uart_puts("BENCH_BANNER\n");
    GPIOA_CRH = (GPIOA_CRH & ~(0xFu << GPIOA_CRH_PA9_SHIFT))
                | (CRH_GPIO_PUSH_PULL_50MHZ << GPIOA_CRH_PA9_SHIFT);
    uart_puts("BENCH_POISON\n");
    uart_init();
    uart_puts("BENCH_UART_OK\n");
#else
    uart_puts("BENCH_BANNER\n");
    uart_puts("BENCH_UART_OK\n");
#endif

    for (;;) {
    }
}
