/* IO-Link MASTER firmware-under-test for the simulated STM32L476.
 *
 * Brings up the real iolinki-master stack over the USART2 PHY and runs its
 * cyclic loop against a real iolinki DEVICE running as firmware on a separate
 * simulated chip, wired C/Q-to-C/Q by a UartCrossLink. Reaches OPERATE and
 * reads the device's process data.
 *
 * Observability for the host-side test harness:
 *   g_master_state  — current iolink_master_state_t (3 == OPERATE)
 *   g_master_pd0    — latest PD-in byte from the device
 * Both live in .data at fixed addresses (see the build's symbol map) and are
 * read via the bus by the integration test; the firmware never has to format a
 * UART message for the test to observe progress.
 */
#include "iolinki_master/master.h"
#include "phy_labwired.h"
#include "debug_uart.h"
#include <stdint.h>

volatile uint8_t g_master_state = 0xFFu; /* 0xFF = not yet initialized */
volatile uint8_t g_master_pd0 = 0xFFu;

int main(void) {
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
    for (;;) {
        iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, now);
        now += 20u; /* 2 ms cycles in 100us units (min_cycle_time) */

        g_master_state = (uint8_t)iolink_master_get_state(&port);

        uint8_t pd[1] = {0u};
        uint8_t n = 0u;
        if (iolink_master_get_pd_in(&port, pd, sizeof(pd), &n) == 0 && n >= 1u) {
            g_master_pd0 = pd[0];
        }

        dbg_hex8(g_master_state); /* also observable on USART1 */
    }
}
