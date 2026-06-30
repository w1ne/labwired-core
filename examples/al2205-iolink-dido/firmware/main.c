/* AL2205-style IO-Link DI device — firmware-under-test.
 *
 * Runs two independent iolinki device contexts on USART2/USART3. Each context
 * publishes process data read from its own simulated 74HC165 input shifter.
 */
#include "iolinki/device.h"
#include "phy_labwired.h"
#include "debug_uart.h"
#include <string.h>
#include <stdint.h>

#define USART2_BASE 0x40004400u
#define USART3_BASE 0x40004800u

/* SPI (stm32_fifo layout) reads the 74HC165 digital-input shift register:
 * one transfer clocks out the 8 input channels as a byte on MISO. */
#define SPI1_BASE 0x40013000u
#define SPI2_BASE 0x40003800u
#define SREG(base, o) (*(volatile uint32_t *)((base) + (o)))
#define CR1_MSTR (1u << 2)
#define CR1_SPE (1u << 6)
#define SR_RXNE (1u << 0)

static void spi_init(uintptr_t spi_base) {
    SREG(spi_base, 0x00u) = CR1_SPE | CR1_MSTR; /* master, enabled, fastest baud */
}

static uint8_t spi_read_byte(uintptr_t spi_base) {
    SREG(spi_base, 0x0Cu) = 0x00u; /* dummy write triggers the transfer */
    for (uint32_t i = 0; i < 100000u; i++) {
        if (SREG(spi_base, 0x08u) & SR_RXNE) {
            return (uint8_t)SREG(spi_base, 0x0Cu);
        }
    }
    return 0u; /* bounded: never hang the IO-Link loop */
}

typedef struct {
    const char *name;
    uintptr_t usart_base;
    uintptr_t spi_base;
    iolink_device_ctx_t device;
    iolink_device_config_t config;
    iolink_dll_state_t last_state;
} iolink_port_t;

static int port_init(iolink_port_t *port) {
    memset(&port->config, 0, sizeof(port->config));
    iolink_phy_labwired_init(&port->config.phy, port->usart_base);
    port->config.stack.m_seq_type = IOLINK_M_SEQ_TYPE_1_1;
    port->config.stack.min_cycle_time = 0;
    port->config.stack.pd_in_len = 1;
    port->config.stack.pd_out_len = 0;
    port->config.stack.t_pd_us = 0;
    port->last_state = (iolink_dll_state_t)0xFF;

    if (iolink_device_init(&port->device, &port->config) != 0) {
        return -1;
    }
    iolink_device_set_timing_enforcement(&port->device, false);
    return 0;
}

static void port_process(iolink_port_t *port, uint8_t pd) {
    iolink_device_pd_input_update(&port->device, &pd, 1, true);
    iolink_device_process(&port->device);

    iolink_dll_state_t state = iolink_device_get_state(&port->device);
    if (state != port->last_state) {
        port->last_state = state;
        dbg_puts(port->name);
        dbg_puts(" STATE=");
        dbg_hex8((unsigned char)state);
        if (state == IOLINK_DLL_STATE_OPERATE) {
            dbg_puts(" OPERATE PD=");
            dbg_hex8(pd);
        }
        dbg_puts("\r\n");
    }
}

int main(void) {
    static iolink_port_t ports[] = {
        {.name = "PORT2", .usart_base = USART2_BASE, .spi_base = SPI1_BASE},
        {.name = "PORT3", .usart_base = USART3_BASE, .spi_base = SPI2_BASE},
    };

    dbg_uart_init();
    dbg_puts("AL2205 BOOT\r\n");

    /* Zero the whole struct first: on this toolchain (arm-none-eabi GCC 10.2,
     * -Os, short-enums) a designated-initializer left t_pd_us uninitialised,
     * which made the stack arm a bogus power-on delay (t_pd) that never
     * expired. memset + explicit assignment is robust. */
    if ((port_init(&ports[0]) != 0) || (port_init(&ports[1]) != 0)) {
        dbg_puts("IOLINK INIT FAIL\r\n");
        for (;;) {
        }
    }
    spi_init(SPI1_BASE);
    spi_init(SPI2_BASE);
    dbg_puts("IOLINK INIT OK\r\n");

    for (;;) {
        /* Read the 8 digital inputs from the 74HC165 and publish them as the
         * IO-Link process data the master cyclically reads. */
        port_process(&ports[0], spi_read_byte(ports[0].spi_base));
        port_process(&ports[1], spi_read_byte(ports[1].spi_base));
        /* Deliberately do NOT advance g_iolink_ticks_ms: the CPU loops far
         * faster than the simulated UART byte rate, so a per-loop tick would
         * race the stack's millisecond timeouts (e.g. the >1000 ms inactivity
         * watchdog resets the link to STARTUP). With the clock frozen and
         * timing enforcement off, the handshake is driven purely by byte
         * arrival, which is what the cycle-stepped simulator models. */
    }
}
