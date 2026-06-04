/* AL2205-style IO-Link DI device — firmware-under-test.
 *
 * M2: bring up the iolinki device stack over the USART2 PHY and run its loop
 * with a constant process-data input. The native IO-Link master (M3) and the
 * 74HC165 input shifter (M4) are wired in later milestones.
 */
#include "iolinki/iolink.h"
#include "iolinki/application.h"
#include "phy_labwired.h"
#include "debug_uart.h"
#include <string.h>
#include <stdint.h>

/* SPI1 (stm32_fifo layout) reads the 74HC165 digital-input shift register:
 * one transfer clocks out the 8 input channels as a byte on MISO. */
#define SPI1_BASE 0x40013000u
#define SREG(o) (*(volatile uint32_t *)(SPI1_BASE + (o)))
#define SPI_CR1 SREG(0x00u)
#define SPI_SR SREG(0x08u)
#define SPI_DR SREG(0x0Cu)
#define CR1_MSTR (1u << 2)
#define CR1_SPE (1u << 6)
#define SR_RXNE (1u << 0)

static void spi1_init(void) {
    SPI_CR1 = CR1_SPE | CR1_MSTR; /* master, enabled, fastest baud */
}

static uint8_t spi1_read_byte(void) {
    SPI_DR = 0x00u; /* dummy write triggers the transfer */
    for (uint32_t i = 0; i < 100000u; i++) {
        if (SPI_SR & SR_RXNE) {
            return (uint8_t)SPI_DR;
        }
    }
    return 0u; /* bounded: never hang the IO-Link loop */
}

int main(void) {
    dbg_uart_init();
    dbg_puts("AL2205 BOOT\r\n");

    /* Zero the whole struct first: on this toolchain (arm-none-eabi GCC 10.2,
     * -Os, short-enums) a designated-initializer left t_pd_us uninitialised,
     * which made the stack arm a bogus power-on delay (t_pd) that never
     * expired. memset + explicit assignment is robust. */
    iolink_config_t cfg;
    memset(&cfg, 0, sizeof(cfg));
    cfg.m_seq_type = IOLINK_M_SEQ_TYPE_1_1;
    cfg.min_cycle_time = 0;
    cfg.pd_in_len = 1;
    cfg.pd_out_len = 0;
    cfg.t_pd_us = 0;
    if (iolink_init(iolink_phy_labwired_get(), &cfg) != 0) {
        dbg_puts("IOLINK INIT FAIL\r\n");
        for (;;) {
        }
    }
    iolink_set_timing_enforcement(false);
    spi1_init();
    dbg_puts("IOLINK INIT OK\r\n");

    iolink_dll_state_t last = (iolink_dll_state_t)0xFF;
    for (;;) {
        /* Read the 8 digital inputs from the 74HC165 and publish them as the
         * IO-Link process data the master cyclically reads. */
        uint8_t pd = spi1_read_byte();
        iolink_pd_input_update(&pd, 1, true);
        iolink_process();
        /* Deliberately do NOT advance g_iolink_ticks_ms: the CPU loops far
         * faster than the simulated UART byte rate, so a per-loop tick would
         * race the stack's millisecond timeouts (e.g. the >1000 ms inactivity
         * watchdog resets the link to STARTUP). With the clock frozen and
         * timing enforcement off, the handshake is driven purely by byte
         * arrival, which is what the cycle-stepped simulator models. */

        iolink_dll_state_t s = iolink_get_state();
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
