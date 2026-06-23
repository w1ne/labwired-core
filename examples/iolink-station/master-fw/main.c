/* IO-Link MASTER firmware-under-test for the simulated STM32L476.
 *
 * Brings up the real iolinki-master stack over the USART2 PHY and runs its
 * cyclic loop against a real iolinki DEVICE running as firmware on a separate
 * simulated chip, wired C/Q-to-C/Q by a UartCrossLink. Reaches OPERATE and
 * reads the device's process data.
 *
 * Built as a standard STM32CubeL4 project (CMSIS startup/system/linker), with
 * peripherals driven through the CMSIS register definitions — no hand-computed
 * register addresses.
 *
 * Observability for the host-side test harness:
 *   g_master_state  — current iolink_master_state_t (3 == OPERATE)
 *   g_master_pd0    — latest PD-in byte from the device
 * The integration test resolves both symbols from the ELF and reads them via
 * the bus; the firmware never has to format a UART message to be observed.
 */
#include "stm32l476xx.h"
#include "iolinki_master/master.h"
#include "phy_labwired.h"
#include "debug_uart.h"
#include <stdint.h>

/* The CMSIS startup calls __libc_init_array for C++/constructor init-array
 * entries; this firmware has none, and -nostartfiles drops the crt object that
 * defines _init, so provide an empty implementation. */
void __libc_init_array(void) {}

volatile uint8_t g_master_state = 0xFFu; /* 0xFF = not yet initialized */
volatile uint8_t g_master_pd0 = 0xFFu;

/* RCC (STM32L4, RM0351 §6.4) — the simulator models clock-gating, so USART1
 * (debug, APB2) and USART2 (IO-Link PHY, APB1) are unclocked out of reset and
 * their registers read/write as no-ops until the matching enable bit is set. */
static void rcc_init(void) {
    RCC->APB2ENR |= RCC_APB2ENR_USART1EN;   /* debug UART */
    RCC->APB1ENR1 |= RCC_APB1ENR1_USART2EN; /* IO-Link C/Q PHY */
}

int main(void) {
    rcc_init();
    dbg_uart_init();

    iolink_master_port_t port;
    iolink_master_config_t cfg = phy_labwired_master_config();
    const iolink_phy_api_t *phy = phy_labwired_master_phy();

    if (iolink_master_init(&port, phy, &cfg) != 0) {
        g_master_state = 0xEEu; /* init failure sentinel */
        for (;;) {
        }
    }

    uint32_t now = 0u;
    uint8_t last_state = 0xFEu; /* force a first print */
    uint8_t last_pd = 0xFEu;
    for (;;) {
        iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, now);
        now += 20u; /* 2 ms cycles in 100us units (min_cycle_time) */

        g_master_state = (uint8_t)iolink_master_get_state(&port);

        uint8_t pd[1] = {0u};
        uint8_t n = 0u;
        if (iolink_master_get_pd_in(&port, pd, sizeof(pd), &n) == 0 && n >= 1u) {
            g_master_pd0 = pd[0];
        }

        /* Print debug on USART1 only on a change — the CPU loops far faster
         * than the IO-Link cycle, so logging every iteration floods the serial
         * monitor with the same byte. Mirrors the device firmware's on-change
         * tracing. (The host test still reads g_master_state from RAM.) */
        if (g_master_state != last_state || g_master_pd0 != last_pd) {
            last_state = g_master_state;
            last_pd = g_master_pd0;
            dbg_puts("STATE=");
            dbg_hex8(g_master_state);
            if (g_master_state == 3u /* OPERATE */) {
                dbg_puts(" PD=");
                dbg_hex8(g_master_pd0);
            }
            dbg_puts("\r\n");
        }
    }
}
