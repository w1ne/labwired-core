/*
 * F103 fidelity benchmark — one source, three firmware variants.
 *
 * The same firmware is compiled three ways to probe whether an emulator models
 * two silicon facts that low-fidelity emulators skip:
 *
 *   control  (default)        : enables the USART1 clock, prints BENCH_UART_OK.
 *                               Correct firmware. Must PASS on real silicon and
 *                               on any emulator. This is the positive control —
 *                               it proves the UART path and harness work.
 *
 *   clockbug (-DSKIP_UART_CLOCK)
 *                             : does the exact same thing but FORGETS to set
 *                               RCC_APB2ENR.USART1EN. On real STM32F103 the
 *                               USART is held in reset with its clock gated, so
 *                               SR.TXE never asserts and nothing is ever
 *                               transmitted. Expected real-hardware result:
 *                               no BENCH_UART_OK (the firmware hangs in the TXE
 *                               poll). An emulator that does not model RCC clock
 *                               gating asserts TXE anyway and prints the marker —
 *                               a false pass.
 *
 *   gpiobug  (-DGPIO_CLOCK_BUG): enables the USART1 clock (so it can report),
 *                               then drives GPIOA WITHOUT enabling the GPIOA
 *                               clock (RCC_APB2ENR.IOPAEN). On real silicon the
 *                               port is held in reset: CRL/ODR writes are
 *                               dropped and the readback never reflects them, so
 *                               BENCH_GPIO_OK never prints. An emulator that does
 *                               not gate GPIO accepts the writes — a false pass.
 *                               A second peripheral, same fidelity gap as
 *                               clockbug: this is not a one-off.
 *
 *   rambug   (-DRAM_OVERFLOW) : enables the clock, then writes one word 4 KB
 *                               past the end of the 20 KB SRAM that an
 *                               STM32F103C8 actually has (0x2000_5000), reads it
 *                               back, and only prints BENCH_RAM_OK if the
 *                               readback matches. On real silicon that address
 *                               is unimplemented: the store faults (HardFault)
 *                               and the marker never prints. An emulator that
 *                               maps an oversized RAM accepts the write, the
 *                               readback matches, and it prints the marker —
 *                               a false pass.
 *
 * Ground truth (from RM0008 / the F103C8 datasheet), what the benchmark scores
 * each emulator against:
 *   control  -> PASS  (BENCH_UART_OK present)
 *   clockbug -> FAIL  (BENCH_UART_OK absent — clock gated, no TX)
 *   rambug   -> FAIL  (BENCH_RAM_OK absent — store faults past 20 KB)
 */

#include <stdint.h>

#define REG32(addr) (*(volatile uint32_t *) (addr))

/* --- RCC (F1): peripheral clock enables (RM0008 §7.3.7) --- */
#define RCC_BASE 0x40021000u
#define RCC_APB2ENR REG32(RCC_BASE + 0x18u)
#define RCC_APB2ENR_USART1EN (1u << 14)
#define RCC_APB2ENR_IOPAEN (1u << 2)

/* --- GPIOA (F1 layout: CRL @ 0x00, ODR @ 0x0C) --- */
#define GPIOA_BASE 0x40010800u
#define GPIOA_CRL REG32(GPIOA_BASE + 0x00u)
#define GPIOA_ODR REG32(GPIOA_BASE + 0x0Cu)

/* --- USART1 (F1 layout: SR @ 0x00, DR @ 0x04, CR1 @ 0x0C) --- */
#define USART1_BASE 0x40013800u
#define U1_SR REG32(USART1_BASE + 0x00u)
#define U1_DR REG32(USART1_BASE + 0x04u)
#define U1_CR1 REG32(USART1_BASE + 0x0Cu)
#define SR_TXE (1u << 7)
#define CR1_UE (1u << 13)
#define CR1_TE (1u << 3)

static void uart_init(void) { U1_CR1 = CR1_UE | CR1_TE; }

static void uart_putc(char c)
{
    /* Real silicon: TXE only asserts once the USART is clocked and enabled.
     * With the clock gated this loop never exits — exactly what real hardware
     * does, and what a faithful emulator must reproduce. */
    while ((U1_SR & SR_TXE) == 0u) {
    }
    U1_DR = (uint32_t) (uint8_t) c;
}

static void uart_puts(const char *s)
{
    while (*s) uart_putc(*s++);
}

int main(void)
{
#ifndef SKIP_UART_CLOCK
    RCC_APB2ENR |= RCC_APB2ENR_USART1EN; /* clockbug omits exactly this line */
#endif
    uart_init();

#ifdef RAM_OVERFLOW
    /* 0x2000_6000 is 4 KB past the end of the F103C8's 20 KB SRAM. */
    volatile uint32_t *oob = (volatile uint32_t *) 0x20006000u;
    *oob = 0xCAFEBABEu;
    uart_puts("BENCH_BANNER\n");
    if (*oob == 0xCAFEBABEu) {
        uart_puts("BENCH_RAM_OK\n"); /* only reachable if the OOB store stuck */
    }
#elif defined(GPIO_CLOCK_BUG)
    /* GPIOA clock deliberately NOT enabled (no RCC_APB2ENR.IOPAEN). */
    uart_puts("BENCH_BANNER\n");
    GPIOA_CRL = 0x33333333u;     /* all low pins = output (dropped if gated) */
    GPIOA_ODR = 0x000000FFu;     /* drive PA0..PA7 high (dropped if gated)   */
    if ((GPIOA_ODR & 0x000000FFu) == 0x000000FFu) {
        uart_puts("BENCH_GPIO_OK\n"); /* readback only reflects a clocked port */
    }
#else
    uart_puts("BENCH_BANNER\n");
    uart_puts("BENCH_UART_OK\n");
#endif

    for (;;) {
    }
}
