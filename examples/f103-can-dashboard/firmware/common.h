/* Shared register-level code for the three STM32F103 (Blue Pill) nodes of the
 * CAN dashboard demo. Plain CMSIS-style register access, no HAL, no libc.
 *
 * Clocks (RM0008 section 7): 8 MHz HSE x 9 (PLL) = 72 MHz SYSCLK = HCLK,
 * APB2 = 72 MHz, APB1 = HCLK / 2 = 36 MHz (timers on APB1 then run at 72 MHz).
 */
#ifndef CAN_DEMO_COMMON_H
#define CAN_DEMO_COMMON_H

#include <stdint.h>

#define REG32(addr) (*(volatile uint32_t *) (addr))

#define __WFI() __asm volatile("wfi")
#define __DSB() __asm volatile("dsb" ::: "memory")

/* ---- RCC / FLASH ------------------------------------------------------- */
#define RCC_BASE 0x40021000u
#define RCC_CR REG32(RCC_BASE + 0x00u)
#define RCC_CFGR REG32(RCC_BASE + 0x04u)
#define RCC_APB2ENR REG32(RCC_BASE + 0x18u)
#define RCC_APB1ENR REG32(RCC_BASE + 0x1Cu)
#define FLASH_ACR REG32(0x40022000u)

#define RCC_APB2ENR_AFIOEN (1u << 0)
#define RCC_APB2ENR_IOPAEN (1u << 2)
#define RCC_APB2ENR_IOPBEN (1u << 3)
#define RCC_APB2ENR_IOPCEN (1u << 4)
#define RCC_APB2ENR_ADC1EN (1u << 9)
#define RCC_APB2ENR_USART1EN (1u << 14)
#define RCC_APB1ENR_TIM2EN (1u << 0)
#define RCC_APB1ENR_TIM3EN (1u << 1)
#define RCC_APB1ENR_I2C1EN (1u << 21)
#define RCC_APB1ENR_CAN1EN (1u << 25)

/* HSE 8 MHz -> PLL x9 -> 72 MHz, APB1 /2, APB2 /1, ADC /6 (12 MHz). */
static inline void clock_init_72mhz(void)
{
    RCC_CR |= (1u << 16); /* HSEON */
    while ((RCC_CR & (1u << 17)) == 0u) { /* HSERDY (startup, once) */
    }
    FLASH_ACR = 0x12u; /* prefetch on, 2 wait states for 72 MHz */
    /* PLLSRC = HSE (bit16), PLLMUL = x9 (0b0111 << 18), PPRE1 = /2 (0b100 << 8),
     * ADCPRE = /6 (0b10 << 14). */
    RCC_CFGR = (1u << 16) | (7u << 18) | (4u << 8) | (2u << 14);
    RCC_CR |= (1u << 24); /* PLLON */
    while ((RCC_CR & (1u << 25)) == 0u) { /* PLLRDY (once) */
    }
    RCC_CFGR |= 2u; /* SW = PLL */
    while (((RCC_CFGR >> 2) & 3u) != 2u) { /* SWS = PLL (once) */
    }
}

/* ---- GPIO (F1 layout: CRL @ 0x00, CRH @ 0x04, 4 bits per pin) ----------- */
#define GPIOA_BASE 0x40010800u
#define GPIOB_BASE 0x40010C00u
#define GPIOC_BASE 0x40011000u
#define GPIO_CRL(base) REG32((base) + 0x00u)
#define GPIO_CRH(base) REG32((base) + 0x04u)
#define GPIO_ODR(base) REG32((base) + 0x0Cu)

#define CNF_ANALOG 0x0u
#define CNF_INPUT_FLOATING 0x4u
#define CNF_OUT_PP_2MHZ 0x2u
#define CNF_AF_PP_50MHZ 0xBu
#define CNF_AF_OD_50MHZ 0xFu

static inline void gpio_crl_set(uint32_t base, unsigned pin, uint32_t cfg)
{
    GPIO_CRL(base) = (GPIO_CRL(base) & ~(0xFu << (pin * 4u))) | (cfg << (pin * 4u));
}
static inline void gpio_crh_set(uint32_t base, unsigned pin, uint32_t cfg)
{
    pin -= 8u;
    GPIO_CRH(base) = (GPIO_CRH(base) & ~(0xFu << (pin * 4u))) | (cfg << (pin * 4u));
}

/* ---- NVIC / timers ------------------------------------------------------ */
#define NVIC_ISER0 REG32(0xE000E100u)
#define IRQ_CAN1_RX0 20u
#define IRQ_TIM2 28u
#define IRQ_TIM3 29u
static inline void nvic_enable(unsigned irq) { NVIC_ISER0 = 1u << irq; }

#define TIM2_BASE 0x40000000u
#define TIM3_BASE 0x40000400u
#define TIM_CR1(b) REG32((b) + 0x00u)
#define TIM_DIER(b) REG32((b) + 0x0Cu)
#define TIM_SR(b) REG32((b) + 0x10u)
#define TIM_EGR(b) REG32((b) + 0x14u)
#define TIM_PSC(b) REG32((b) + 0x28u)
#define TIM_ARR(b) REG32((b) + 0x2Cu)

/* Periodic update interrupt every `period_100us` x 100 us. The APB1 timer
 * clock is 2 x 36 MHz = 72 MHz (RM0008 section 7.2), so PSC 7199 gives 10 kHz. */
static inline void timer_start_100us(uint32_t base, uint32_t period_100us)
{
    TIM_PSC(base) = 7199u;
    TIM_ARR(base) = period_100us - 1u;
    TIM_EGR(base) = 1u; /* UG: load PSC/ARR */
    TIM_SR(base) = 0u;  /* drop the UIF the UG raised */
    TIM_DIER(base) = 1u; /* UIE */
    TIM_CR1(base) |= 1u; /* CEN */
}

/* ---- USART1 on PA9 (TX only) -------------------------------------------- */
#define USART1_BASE 0x40013800u
#define U1_SR REG32(USART1_BASE + 0x00u)
#define U1_DR REG32(USART1_BASE + 0x04u)
#define U1_BRR REG32(USART1_BASE + 0x08u)
#define U1_CR1 REG32(USART1_BASE + 0x0Cu)

static inline void uart_init_115200(void)
{
    RCC_APB2ENR |= RCC_APB2ENR_IOPAEN | RCC_APB2ENR_USART1EN | RCC_APB2ENR_AFIOEN;
    gpio_crh_set(GPIOA_BASE, 9u, CNF_AF_PP_50MHZ);
    U1_BRR = 72000000u / 115200u; /* PCLK2 = 72 MHz -> 625 */
    U1_CR1 = (1u << 13) | (1u << 3); /* UE | TE */
}
static inline void uart_putc(char c)
{
    while ((U1_SR & (1u << 7)) == 0u) { /* TXE */
    }
    U1_DR = (uint32_t) (uint8_t) c;
}
static inline void uart_puts(const char *s)
{
    while (*s != '\0') {
        uart_putc(*s++);
    }
}
static inline void uart_put_uint(uint32_t v)
{
    char tmp[10];
    unsigned n = 0u;
    do {
        tmp[n++] = (char) ('0' + v % 10u);
        v /= 10u;
    } while (v != 0u);
    while (n != 0u) {
        uart_putc(tmp[--n]);
    }
}

/* ---- bxCAN1 on PA11 (RX) / PA12 (TX), RM0008 section 24 ------------------ */
#define CAN_BASE 0x40006400u
#define CAN_MCR REG32(CAN_BASE + 0x000u)
#define CAN_MSR REG32(CAN_BASE + 0x004u)
#define CAN_TSR REG32(CAN_BASE + 0x008u)
#define CAN_RF0R REG32(CAN_BASE + 0x00Cu)
#define CAN_IER REG32(CAN_BASE + 0x014u)
#define CAN_ESR REG32(CAN_BASE + 0x018u)
#define CAN_BTR REG32(CAN_BASE + 0x01Cu)
#define CAN_TIR(mb) REG32(CAN_BASE + 0x180u + (mb) * 0x10u)
#define CAN_TDTR(mb) REG32(CAN_BASE + 0x184u + (mb) * 0x10u)
#define CAN_TDLR(mb) REG32(CAN_BASE + 0x188u + (mb) * 0x10u)
#define CAN_TDHR(mb) REG32(CAN_BASE + 0x18Cu + (mb) * 0x10u)
#define CAN_RIR REG32(CAN_BASE + 0x1B0u)
#define CAN_RDTR REG32(CAN_BASE + 0x1B4u)
#define CAN_RDLR REG32(CAN_BASE + 0x1B8u)
#define CAN_RDHR REG32(CAN_BASE + 0x1BCu)
#define CAN_FMR REG32(CAN_BASE + 0x200u)
#define CAN_FM1R REG32(CAN_BASE + 0x204u)
#define CAN_FS1R REG32(CAN_BASE + 0x20Cu)
#define CAN_FFA1R REG32(CAN_BASE + 0x214u)
#define CAN_FA1R REG32(CAN_BASE + 0x21Cu)
#define CAN_FR1(bank) REG32(CAN_BASE + 0x240u + (bank) * 8u)
#define CAN_FR2(bank) REG32(CAN_BASE + 0x244u + (bank) * 8u)

#define CAN_MCR_INRQ (1u << 0)
#define CAN_MCR_SLEEP (1u << 1)
#define CAN_MSR_INAK (1u << 0)
#define CAN_IER_FMPIE0 (1u << 1)
#define CAN_TSR_TME0 (1u << 26)
#define CAN_RF0R_RFOM0 (1u << 5)
#define CAN_ESR_EWGF (1u << 0)
#define CAN_ESR_EPVF (1u << 1)
#define CAN_ESR_BOFF (1u << 2)
#define CAN_TIR_TXRQ (1u << 0)

/* Bit timing from the bitrate: tq = (BRP + 1) / 36 MHz, one bit = 18 tq
 * (SYNC 1 + TS1 14 + TS2 3), sample point at 15/18 = 83 %.
 *   500 kbit/s: BRP = 36e6 / (500000 x 18) - 1 = 3
 *   250 kbit/s: BRP = 7 */
#define CAN_PCLK1_HZ 36000000u
#define CAN_TQ_PER_BIT 18u
#define CAN_BRP_FOR(bitrate) ((CAN_PCLK1_HZ / ((bitrate) *CAN_TQ_PER_BIT)) - 1u)
#define CAN_BTR_FOR(bitrate) \
    ((0u << 24) | (2u << 20) | (13u << 16) | CAN_BRP_FOR(bitrate)) /* SJW 1, TS2 3, TS1 14 */

/* Bring bxCAN1 up in normal mode at `bitrate`. `list_id0` / `list_id1` (11-bit
 * IDs, or 0xFFFFFFFF for none) become a 32-bit identifier-list filter into
 * FIFO0; with none given, a mask filter that accepts everything is used.
 * Automatic retransmission stays on (MCR.NART = 0), ABOM stays off. */
static inline void can_init(uint32_t btr, uint32_t list_id0, uint32_t list_id1)
{
    RCC_APB2ENR |= RCC_APB2ENR_IOPAEN | RCC_APB2ENR_AFIOEN;
    RCC_APB1ENR |= RCC_APB1ENR_CAN1EN;
    gpio_crh_set(GPIOA_BASE, 11u, CNF_INPUT_FLOATING); /* CAN_RX */
    gpio_crh_set(GPIOA_BASE, 12u, CNF_AF_PP_50MHZ);    /* CAN_TX */

    CAN_MCR = (CAN_MCR & ~CAN_MCR_SLEEP) | CAN_MCR_INRQ;
    while ((CAN_MSR & CAN_MSR_INAK) == 0u) { /* leave sleep, enter init (once) */
    }
    CAN_BTR = btr;

    CAN_FMR |= 1u; /* FINIT */
    CAN_FS1R = 1u; /* bank 0: 32-bit scale */
    CAN_FFA1R = 0u; /* bank 0 -> FIFO0 */
    if (list_id0 == 0xFFFFFFFFu) {
        CAN_FM1R = 0u; /* mask mode, mask 0 = accept all */
        CAN_FR1(0) = 0u;
        CAN_FR2(0) = 0u;
    } else {
        CAN_FM1R = 1u; /* identifier list */
        CAN_FR1(0) = list_id0 << 21;
        CAN_FR2(0) = (list_id1 == 0xFFFFFFFFu ? list_id0 : list_id1) << 21;
    }
    CAN_FA1R = 1u;
    CAN_FMR &= ~1u;

    CAN_MCR &= ~CAN_MCR_INRQ;
    while ((CAN_MSR & CAN_MSR_INAK) != 0u) { /* join the bus (once) */
    }
}

/* Queue a standard-ID data frame of 1..8 bytes in a free mailbox. Returns 0 if
 * all three mailboxes are busy (the frame is dropped; a later period retries
 * with fresh data). */
static inline int can_send(uint32_t id, uint32_t lo, uint32_t hi, uint32_t dlc)
{
    uint32_t tsr = CAN_TSR;
    uint32_t mb;
    if ((tsr & (1u << 26)) != 0u) {
        mb = 0u;
    } else if ((tsr & (1u << 27)) != 0u) {
        mb = 1u;
    } else if ((tsr & (1u << 28)) != 0u) {
        mb = 2u;
    } else {
        return 0;
    }
    CAN_TDTR(mb) = dlc;
    CAN_TDLR(mb) = lo;
    CAN_TDHR(mb) = hi;
    CAN_TIR(mb) = (id << 21) | CAN_TIR_TXRQ;
    return 1;
}

#endif
