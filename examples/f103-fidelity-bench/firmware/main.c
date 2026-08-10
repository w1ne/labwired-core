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

/* Mux PA9 and program the divisor, THEN enable the transmitter.
 *
 * `U1_CR1 = CR1_UE | CR1_TE` was the whole of this. That transmits on
 * LabWired's permissive USART model and nowhere else: PA9 stays the floating
 * input it is after reset, so the pad route never goes live and a logic
 * analyzer on PA9 reads the GPIO output latch instead of the serial waveform.
 * A zero BRR is the other half — no divisor means no bit period, so there is
 * nothing to narrate even once a route exists.
 *
 * Under GPIO_CLOCK_BUG the CRH write below is DROPPED, because that variant
 * deliberately never enables RCC_APB2ENR.IOPAEN. That is the bench working as
 * designed, not a regression: an unclocked port must swallow the write. */
static void uart_init(void)
{
    GPIOA_CRH = (GPIOA_CRH & ~(0xFu << GPIOA_CRH_PA9_SHIFT))
                | (CRH_AF_PUSH_PULL_50MHZ << GPIOA_CRH_PA9_SHIFT);
    U1_BRR = U1_BRR_115200_AT_8MHZ;
    U1_CR1 = CR1_UE | CR1_TE;
}

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
#ifndef GPIO_CLOCK_BUG
    /* PA9 must be clocked before uart_init() can mux it onto USART1_TX.
     * gpiobug omits exactly this line, and must keep omitting it: an UNCLOCKED
     * GPIOA is the whole point of that variant. */
    RCC_APB2ENR |= RCC_APB2ENR_IOPAEN;
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
