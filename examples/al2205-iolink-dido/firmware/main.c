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

/* Defined by iolinki's baremetal time_utils.c; advanced here in the loop so the
 * stack's millisecond clock progresses without relying on SysTick interrupts. */
extern volatile unsigned int g_iolink_ticks_ms;

int main(void) {
    dbg_uart_init();
    dbg_puts("AL2205 BOOT\r\n");

    iolink_config_t cfg = {
        .m_seq_type = IOLINK_M_SEQ_TYPE_1_1,
        .min_cycle_time = 0,
        .pd_in_len = 1,
        .pd_out_len = 0,
        .t_pd_us = 0,
    };
    if (iolink_init(iolink_phy_labwired_get(), &cfg) != 0) {
        dbg_puts("IOLINK INIT FAIL\r\n");
        for (;;) {
        }
    }
    iolink_set_timing_enforcement(false);
    dbg_puts("IOLINK INIT OK\r\n");

    uint8_t pd = 0x00;
    for (;;) {
        iolink_pd_input_update(&pd, 1, true);
        iolink_process();
        g_iolink_ticks_ms++;
    }
}
