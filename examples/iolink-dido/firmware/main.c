/* IO-Link DI/DO IO-Link DI device — firmware-under-test.
 *
 * M2: bring up the iolinki device stack over the USART2 PHY and run its loop
 * with a constant process-data input. The native IO-Link master (M3) and the
 * 74HC165 input shifter (M4) are wired in later milestones.
 *
 * Built as a standard STM32CubeL4 project: the ST CMSIS device pack supplies
 * the startup code, system_stm32l4xx.c and the NUCLEO-L476RG linker script, and
 * peripherals are driven through the CMSIS register definitions (RCC->, USARTx->,
 * SPIx->) — no hand-computed register addresses.
 */
#include "stm32l476xx.h"
#include "iolinki/application.h"
#include "iolinki/device.h"
#include "iolinki/iolink.h"
#include "phy_labwired.h"
#include "debug_uart.h"
#include <string.h>
#include <stdint.h>

/* The CMSIS startup calls __libc_init_array to run C++/constructor init-array
 * entries; this firmware has none, and -nostartfiles drops the crt object that
 * defines _init, so provide an empty implementation (matches the other LabWired
 * Cube examples). Plain C globals are initialised by the startup .data copy. */
void __libc_init_array(void) {}

/* RCC (STM32L4, RM0351 §6.4) — enable the peripheral clocks this firmware uses
 * before touching their registers. REQUIRED on real silicon and modelled by the
 * simulator (clock-gating): USART1/USART2/SPI1 are unclocked out of reset, so
 * their register writes are dropped and ISR.TXE/SR.RXNE never assert until the
 * matching RCC enable bit is set. USART1/SPI1 are on APB2, USART2 on APB1. */
static void rcc_init(void) {
    RCC->APB2ENR |= RCC_APB2ENR_SPI1EN | RCC_APB2ENR_USART1EN;
    RCC->APB1ENR1 |= RCC_APB1ENR1_USART2EN;
}

/* SPI1 (stm32_fifo layout) reads the 74HC165 digital-input shift register:
 * one transfer clocks out the 8 input channels as a byte on MISO. */
static void spi1_init(void) {
    SPI1->CR1 = SPI_CR1_SPE | SPI_CR1_MSTR; /* master, enabled, fastest baud */
}

static uint8_t spi1_read_byte(void) {
    SPI1->DR = 0x00u; /* dummy write triggers the transfer */
    for (uint32_t i = 0; i < 100000u; i++) {
        if (SPI1->SR & SPI_SR_RXNE) {
            return (uint8_t)SPI1->DR;
        }
    }
    return 0u; /* bounded: never hang the IO-Link loop */
}

int main(void) {
    rcc_init();
    dbg_uart_init();
    dbg_puts("IOLINK DIDO BOOT\r\n");

    /* Zero the whole struct first: on this toolchain (arm-none-eabi GCC 10.2,
     * -Os, short-enums) a designated-initializer left t_pd_us uninitialised,
     * which made the stack arm a bogus power-on delay (t_pd) that never
     * expired. memset + explicit assignment is robust. */
    iolink_device_ctx_t device;
    iolink_device_config_t cfg;
    memset(&device, 0, sizeof(device));
    memset(&cfg, 0, sizeof(cfg));
    cfg.phy = *iolink_phy_labwired_get();
    cfg.stack.m_seq_type = IOLINK_M_SEQ_TYPE_1_1;
    cfg.stack.min_cycle_time = 0;
    cfg.stack.pd_in_len = 1;
    cfg.stack.pd_out_len = 0;
    cfg.stack.t_pd_us = 0;
    if (iolink_device_init(&device, &cfg) != 0) {
        dbg_puts("IOLINK INIT FAIL\r\n");
        for (;;) {
        }
    }
    iolink_device_set_timing_enforcement(&device, false);
    spi1_init();
    dbg_puts("IOLINK INIT OK\r\n");

    iolink_dll_state_t last = (iolink_dll_state_t)0xFF;
    for (;;) {
        /* Read the 8 digital inputs from the 74HC165 and publish them as the
         * IO-Link process data the master cyclically reads. */
        uint8_t pd = spi1_read_byte();
        iolink_device_pd_input_update(&device, &pd, 1, true);
        iolink_device_process(&device);
        /* Deliberately do NOT advance g_iolink_ticks_ms: the CPU loops far
         * faster than the simulated UART byte rate, so a per-loop tick would
         * race the stack's millisecond timeouts (e.g. the >1000 ms inactivity
         * watchdog resets the link to STARTUP). With the clock frozen and
         * timing enforcement off, the handshake is driven purely by byte
         * arrival, which is what the cycle-stepped simulator models. */

        iolink_dll_state_t s = iolink_device_get_state(&device);
        if (s != last) {
            last = s;
            /* Trace transitions (so a stall is visible); flag OPERATE for the gate. */
            dbg_puts("STATE=");
            dbg_hex8((unsigned char)s);
            if (s == IOLINK_DLL_STATE_OPERATE) {
                dbg_puts(" OPERATE PD=");
                dbg_hex8(pd);
            }
            dbg_puts("\r\n");
        }
    }
}
