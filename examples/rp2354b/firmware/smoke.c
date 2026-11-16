// SPDX-License-Identifier: MIT
// Original observable smoke using unmodified Raspberry Pi SDK 2.3.1 headers.
#include <stdint.h>
#define _u(x) x##u
#include "hardware/regs/addressmap.h"
#include "hardware/regs/clocks.h"
#include "hardware/regs/io_bank0.h"
#include "hardware/regs/pads_bank0.h"
#include "hardware/regs/pll.h"
#include "hardware/regs/resets.h"
#include "hardware/regs/sio.h"
#include "hardware/regs/uart.h"
#include "hardware/regs/xosc.h"

#define REG(base, offset) (*(volatile uint32_t *)((uintptr_t)(base) + (offset)))
extern uint32_t _stack_top, _sidata, _sdata, _edata, _sbss, _ebss;
static void Default_Handler(void) __attribute__((noreturn));
void Reset_Handler(void) __attribute__((noreturn));
__attribute__((used, section(".vectors")))
static const uintptr_t vectors[16] = {
    (uintptr_t)&_stack_top, (uintptr_t)Reset_Handler,
    (uintptr_t)Default_Handler, (uintptr_t)Default_Handler,
    (uintptr_t)Default_Handler, (uintptr_t)Default_Handler,
    (uintptr_t)Default_Handler, 0, 0, 0, 0,
    (uintptr_t)Default_Handler, (uintptr_t)Default_Handler, 0,
    (uintptr_t)Default_Handler, (uintptr_t)Default_Handler
};
static void Default_Handler(void) {
    for (;;) __asm volatile("wfi");
}
static void release_resets(uint32_t mask) {
    REG(RESETS_BASE, RESETS_RESET_OFFSET + 0x3000u) = mask;
    while ((REG(RESETS_BASE, RESETS_RESET_DONE_OFFSET) & mask) != mask) {}
}
static void clocks_init(void) {
    REG(XOSC_BASE, XOSC_CTRL_OFFSET) =
        (XOSC_CTRL_ENABLE_VALUE_DISABLE << XOSC_CTRL_ENABLE_LSB) |
        XOSC_CTRL_FREQ_RANGE_VALUE_1_15MHZ;
    // ceil(12 MHz / (1000 * 256)): the explicit nominal 1 ms SDK delay.
    REG(XOSC_BASE, XOSC_STARTUP_OFFSET) = 47;
    REG(XOSC_BASE, XOSC_CTRL_OFFSET) =
        (XOSC_CTRL_ENABLE_VALUE_ENABLE << XOSC_CTRL_ENABLE_LSB) |
        XOSC_CTRL_FREQ_RANGE_VALUE_1_15MHZ;
    while (!(REG(XOSC_BASE, XOSC_STATUS_OFFSET) & XOSC_STATUS_STABLE_BITS)) {}
    REG(CLOCKS_BASE, CLOCKS_CLK_REF_DIV_OFFSET) = 1u << 16;
    REG(CLOCKS_BASE, CLOCKS_CLK_REF_CTRL_OFFSET) = CLOCKS_CLK_REF_CTRL_SRC_VALUE_XOSC_CLKSRC;
    while (!(REG(CLOCKS_BASE, CLOCKS_CLK_REF_SELECTED_OFFSET) & 4u)) {}
    release_resets(RESETS_RESET_PLL_SYS_BITS);
    REG(PLL_SYS_BASE, PLL_CS_OFFSET) = 1;
    REG(PLL_SYS_BASE, PLL_FBDIV_INT_OFFSET) = 125;
    REG(PLL_SYS_BASE, PLL_PWR_OFFSET + 0x3000u) = PLL_PWR_PD_BITS | PLL_PWR_VCOPD_BITS;
    while (!(REG(PLL_SYS_BASE, PLL_CS_OFFSET) & PLL_CS_LOCK_BITS)) {}
    REG(PLL_SYS_BASE, PLL_PRIM_OFFSET) =
        (5u << PLL_PRIM_POSTDIV1_LSB) | (2u << PLL_PRIM_POSTDIV2_LSB);
    REG(PLL_SYS_BASE, PLL_PWR_OFFSET + 0x3000u) = PLL_PWR_POSTDIVPD_BITS;
    // 12 MHz * 125 / (5 * 2) = 150 MHz; then CLK_PERI = CLK_SYS.
    REG(CLOCKS_BASE, CLOCKS_CLK_SYS_DIV_OFFSET) = 1u << 16;
    REG(CLOCKS_BASE, CLOCKS_CLK_SYS_CTRL_OFFSET) = CLOCKS_CLK_SYS_CTRL_SRC_VALUE_CLK_REF;
    REG(CLOCKS_BASE, CLOCKS_CLK_SYS_CTRL_OFFSET) = CLOCKS_CLK_SYS_CTRL_SRC_VALUE_CLKSRC_CLK_SYS_AUX;
    while (!(REG(CLOCKS_BASE, CLOCKS_CLK_SYS_SELECTED_OFFSET) & 2u)) {}
    REG(CLOCKS_BASE, CLOCKS_CLK_PERI_DIV_OFFSET) = 1u << 16;
    REG(CLOCKS_BASE, CLOCKS_CLK_PERI_CTRL_OFFSET) = CLOCKS_CLK_PERI_CTRL_ENABLE_BITS |
        (CLOCKS_CLK_PERI_CTRL_AUXSRC_VALUE_CLK_SYS << CLOCKS_CLK_PERI_CTRL_AUXSRC_LSB);
}
static void io_init(void) {
    release_resets(RESETS_RESET_IO_BANK0_BITS | RESETS_RESET_PADS_BANK0_BITS | RESETS_RESET_UART0_BITS);
    uint32_t pad = PADS_BANK0_GPIO0_IE_BITS | PADS_BANK0_GPIO0_SCHMITT_BITS |
        (PADS_BANK0_GPIO0_DRIVE_VALUE_4MA << PADS_BANK0_GPIO0_DRIVE_LSB);
    REG(PADS_BANK0_BASE, PADS_BANK0_GPIO0_OFFSET) = pad;
    REG(PADS_BANK0_BASE, PADS_BANK0_GPIO47_OFFSET) = pad;
    REG(IO_BANK0_BASE, IO_BANK0_GPIO0_CTRL_OFFSET) = IO_BANK0_GPIO0_CTRL_FUNCSEL_VALUE_UART0_TX;
    REG(IO_BANK0_BASE, IO_BANK0_GPIO47_CTRL_OFFSET) = IO_BANK0_GPIO47_CTRL_FUNCSEL_VALUE_SIO_47;
    REG(SIO_BASE, SIO_GPIO_HI_OUT_CLR_OFFSET) = 1u << 15;
    REG(SIO_BASE, SIO_GPIO_HI_OE_SET_OFFSET) = 1u << 15;
    REG(UART0_BASE, UART_UARTCR_OFFSET) = 0;
    // 150 MHz / (16 * (9 + 24/64)) = 1 Mbaud, 8 data bits, no parity.
    REG(UART0_BASE, UART_UARTIBRD_OFFSET) = 9;
    REG(UART0_BASE, UART_UARTFBRD_OFFSET) = 24;
    REG(UART0_BASE, UART_UARTLCR_H_OFFSET) = UART_UARTLCR_H_WLEN_BITS | UART_UARTLCR_H_FEN_BITS;
    REG(UART0_BASE, UART_UARTCR_OFFSET) = UART_UARTCR_UARTEN_BITS | UART_UARTCR_TXE_BITS;
}
static void puts_uart(const char *text) {
    while (*text) {
        while (REG(UART0_BASE, UART_UARTFR_OFFSET) & UART_UARTFR_TXFF_BITS) {}
        REG(UART0_BASE, UART_UARTDR_OFFSET) = (uint8_t)*text++;
    }
}
void Reset_Handler(void) {
    uint32_t *src = &_sidata;
    for (uint32_t *dst = &_sdata; dst < &_edata;) *dst++ = *src++;
    for (uint32_t *dst = &_sbss; dst < &_ebss;) *dst++ = 0;
    clocks_init();
    io_init();
    puts_uart("RP2354B UART OK\n");
    for (unsigned edge = 0; edge < 16; ++edge) {
        REG(SIO_BASE, SIO_GPIO_HI_OUT_XOR_OFFSET) = 1u << 15;
        for (volatile unsigned delay = 0; delay < 256; ++delay) __asm volatile("nop");
    }
    puts_uart("GPIO47 OK\n");
    for (;;) __asm volatile("wfi");
}
