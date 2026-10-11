/* The STM32 F1/F4 legacy I2C1 (CR1/CR2/OAR1/DR/SR1/SR2/CCR/TRISE) on PB6
 * SCL, PB7 SDA, open drain, for the bus-over-nets firmware. Build with
 * -DCHIP_F1 (STM32F103, 72 MHz) or -DCHIP_F4 (STM32F401, 84 MHz). */
#ifndef LEGACY_I2C_H
#define LEGACY_I2C_H
#if defined(CHIP_F1)
#include "f1.h"
#define STACK_TOP 0x20005000u
#define PCLK1_MHZ 36u
#elif defined(CHIP_F4)
#include "f4.h"
#define STACK_TOP 0x20009000u
#define PCLK1_MHZ 42u
#else
#error "build with -DCHIP_F1 or -DCHIP_F4"
#endif

#define CR1_PE    (1u << 0)
#define CR1_START (1u << 8)
#define CR1_STOP  (1u << 9)
#define CR1_ACK   (1u << 10)
#define CR1_POS   (1u << 11)
#define SR1_SB    (1u << 0)
#define SR1_ADDR  (1u << 1)
#define SR1_BTF   (1u << 2)
#define SR1_STOPF (1u << 4)
#define SR1_RXNE  (1u << 6)
#define SR1_TXE   (1u << 7)
#define SR1_AF    (1u << 10)
#define SR2_TRA   (1u << 2)

/* PB6/PB7 to I2C1, open drain; the pull-up is the net's. */
static inline void i2c1_pins(void) {
#if defined(CHIP_F1)
    RCC_APB2ENR |= (1u << 3) | 1u;      /* IOPB, AFIO */
    /* CNF = 11 (AF open drain), MODE = 11 (50 MHz) on pins 6 and 7. */
    GPIOB_CRL = (GPIOB_CRL & ~0xFF000000u) | 0xFF000000u;
#else
    RCC_AHB1ENR |= 2u;                  /* GPIOB */
    GPIOB_OTYPER |= (1u << 6) | (1u << 7);
    for (unsigned pin = 6; pin <= 7; ++pin) {
        set_af_low(&GPIOB_AFRL, pin, 4);
        set_mode(&GPIOB_MODER, pin, 2);
    }
#endif
    RCC_APB1ENR |= 1u << 21;            /* I2C1 */
}

/* Standard mode, 100 kHz: CCR = PCLK1 / 200 kHz. */
static inline void i2c1_timing(void) {
    I2C1_CR2 = (I2C1_CR2 & ~0x3Fu) | PCLK1_MHZ;
    I2C1_CCR = PCLK1_MHZ * 5u;
    I2C1_TRISE = PCLK1_MHZ + 1u;
}
#endif
