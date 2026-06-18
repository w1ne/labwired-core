/* 4-port IO-Link MASTER firmware: one iolinki-master controller driving four
 * ports (USART2/3/4/5), each wired to its own sensor chip running the real
 * device firmware. Reaches OPERATE per port and reads each sensor's PD.
 *
 * Observability (read by the integration test via the bus):
 *   g_master_state[4] @ 0x20000000 — per-port state (3 == OPERATE)
 *   g_master_pd[4]    @ 0x20000004 — per-port latest PD-in byte
 */
#include "iolinki_master/master.h"
#include "phy_labwired.h"
#include "debug_uart.h"
#include <stdint.h>

volatile uint8_t g_master_state[LW_MASTER_PORTS] = {0xFFu, 0xFFu, 0xFFu, 0xFFu};
volatile uint8_t g_master_pd[LW_MASTER_PORTS] = {0xFFu, 0xFFu, 0xFFu, 0xFFu};

int main(void) {
    dbg_uart_init();

    iolink_master_controller_t ctrl;
    iolink_master_port_t ports[LW_MASTER_PORTS];
    iolink_phy_api_t phys[LW_MASTER_PORTS];
    iolink_master_config_t cfgs[LW_MASTER_PORTS];
    phy_labwired_fill(phys, cfgs);

    if (iolink_master_controller_init(&ctrl, ports, LW_MASTER_PORTS, phys, cfgs) != 0) {
        g_master_state[0] = 0xEEu;
        for (;;) {
        }
    }

    uint32_t now = 0u;
    for (;;) {
        iolink_master_controller_tick_at(&ctrl, now);
        now += 20u;
        for (uint8_t i = 0; i < LW_MASTER_PORTS; i++) {
            iolink_master_port_t *p = 0;
            if (iolink_master_controller_get_port(&ctrl, i, &p) == 0 && p) {
                g_master_state[i] = (uint8_t)iolink_master_get_state(p);
                uint8_t pd[1] = {0u};
                uint8_t n = 0u;
                if (iolink_master_get_pd_in(p, pd, sizeof(pd), &n) == 0 && n >= 1u) {
                    g_master_pd[i] = pd[0];
                }
            }
        }
    }
}
