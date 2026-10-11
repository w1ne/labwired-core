/* RP2040 registers the bus-over-nets firmware uses (RP2040 datasheet). */
#ifndef RP2040_H
#define RP2040_H
#include <stdint.h>

#define REG(a) (*(volatile uint32_t *)(a))
#define IO_CTRL(n)  REG(0x40014004u + 8u * (n))    /* GPIOn_CTRL */
#define SSPCR0      REG(0x4003C000u)
#define SSPCR1      REG(0x4003C004u)
#define SSPDR       REG(0x4003C008u)
#define SSPSR       REG(0x4003C00Cu)
#define SSPCPSR     REG(0x4003C010u)

#define IC_CON           REG(0x40044000u)
#define IC_TAR           REG(0x40044004u)
#define IC_SAR           REG(0x40044008u)
#define IC_DATA_CMD      REG(0x40044010u)
#define IC_SS_SCL_HCNT   REG(0x40044014u)
#define IC_SS_SCL_LCNT   REG(0x40044018u)
#define IC_RAW_INTR_STAT REG(0x40044034u)
#define IC_RX_TL         REG(0x40044038u)
#define IC_CLR_RD_REQ    REG(0x40044050u)
#define IC_CLR_TX_ABRT   REG(0x40044054u)
#define IC_CLR_STOP_DET  REG(0x40044060u)
#define IC_ENABLE        REG(0x4004406Cu)
#define IC_RXFLR         REG(0x40044078u)
#define IC_TX_ABRT_SOURCE REG(0x40044080u)

#define FUNC_SPI 1u
#define FUNC_I2C 3u
#define INTR_RX_FULL  (1u << 2)
#define INTR_RD_REQ   (1u << 5)
#define INTR_TX_ABRT  (1u << 6)
#define INTR_STOP_DET (1u << 9)
#define CMD_READ    (1u << 8)
#define CMD_STOP    (1u << 9)
#define CMD_RESTART (1u << 10)
#define FIRST_DATA_BYTE (1u << 11)
#define SR_TFE (1u << 0)
#define SR_RNE (1u << 2)
#define SR_BSY (1u << 4)
#define CR1_SSE (1u << 1)
#define CR1_MS  (1u << 2)
#define CR0_SPO (1u << 6)
#define CR0_SPH (1u << 7)

/* Results the tests read, by address. */
#define RESULT ((volatile uint32_t *)0x20000100u)

static inline void delay(uint32_t loops) {
    for (volatile uint32_t i = 0; i < loops; ++i) {}
}

void reset(void);
void default_handler(void) { for (;;) {} }

__attribute__((section(".vectors"), used))
const uintptr_t vectors[16 + 26] = {
    [0] = 0x20040000u,
    [1] = (uintptr_t)reset,
    [2] = (uintptr_t)default_handler,
    [3] = (uintptr_t)default_handler,
};
#endif
