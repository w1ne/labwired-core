/* 4-port IO-Link MASTER firmware: one iolinki-master controller driving four
 * ports (USART2/3/4/5), each wired to its own sensor chip running the real
 * device firmware. Reaches OPERATE per port and reads each sensor's PD.
 *
 * Built as a standard STM32CubeL4 project (CMSIS startup/system/linker), with
 * peripherals driven through the CMSIS register definitions.
 *
 * Observability (the integration test resolves these symbols from the ELF and
 * reads them via the bus):
 *   g_master_state[4] — per-port state (3 == OPERATE)
 *   g_master_pd[4]    — per-port latest PD-in byte
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

volatile uint8_t g_master_state[LW_MASTER_PORTS] = {0xFFu, 0xFFu, 0xFFu, 0xFFu};
volatile uint8_t g_master_pd[LW_MASTER_PORTS] = {0xFFu, 0xFFu, 0xFFu, 0xFFu};

/* RCC (STM32L4, RM0351 §6.4) — the simulator models clock-gating, so USART1
 * (debug, APB2) and USART2 (IO-Link port 0, APB1) are unclocked out of reset
 * and their registers read/write as no-ops until the matching enable bit is
 * set. USART3/UART4/UART5 (ports 1-3) are not gated by the L476 model, but we
 * enable them too so the firmware matches real silicon. Without enabling
 * USART2, the port-0 PHY busy-waits on ISR.TXE forever and the master never
 * progresses past wake-up. */
static void rcc_init(void) {
    RCC->APB2ENR |= RCC_APB2ENR_USART1EN; /* debug UART */
    RCC->APB1ENR1 |= RCC_APB1ENR1_USART2EN | RCC_APB1ENR1_USART3EN |
                     RCC_APB1ENR1_UART4EN | RCC_APB1ENR1_UART5EN; /* IO-Link ports */
}

int main(void) {
    rcc_init();
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
