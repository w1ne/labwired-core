/* IO-Link SERVICE-SCRIPT MASTER firmware-under-test for the simulated STM32L476.
 *
 * Brings up the real iolinki-master stack over the USART2 PHY against the
 * service-rich device (device-fw-svc). After OPERATE it runs a phased script
 * that drives every major iolinki-master service on the wire and mirrors each
 * result into a volatile global the Rust integration test reads by ELF symbol:
 *   ISDU read (vendor name) -> PD-output echo -> event trigger+read ->
 *   data-storage write+readback -> done. It also mirrors master diagnostics and,
 *   on ERROR, restarts the port so fault-recovery tests can observe a cycle.
 *
 * -------------------------------------------------------------------------
 * Open API questions resolved by reading source before writing this firmware
 * (see the task report for file:line evidence):
 *
 * (1) RESTART API: iolink_master_restart(port) recovers a port from ERROR
 *     (master.h:206, "Returns OK or INVALID_ARG"). Re-init is NOT required.
 *
 * (2) M-SEQ: TYPE_2_2 carries one PD octet in each direction and one
 *     on-request-data octet. READ requests omit OD; WRITE requests carry it.
 *     Both fixtures use the same type and the pinned device supports this
 *     direction-aware stream framing. TYPE_1_1 retains a legacy device layout.
 *
 * (3) EVENT DETAILS: iolink_master_read_event_details reads event memory on
 *     the DIAGNOSIS channel. The result contains qualifier and EventCode
 *     fields, which the firmware exposes to the integration test.
 *
 * (4) pd_output_read RETURN: iolink_device_pd_output_read returns the byte
 *     count copied (device.c:202 returns read_len), NOT 0-for-success (that is
 *     handled in the device firmware, not here).
 *
 * (5) DS RECORD FORMAT: each Data-Storage record is
 *     [Index(2,BE)][Subindex(1)][Length(1)][Data(Length)] (data_storage.h:26,
 *     DS_RECORD_HEADER=4 in data_storage.c:37). A DS write must therefore be a
 *     well-formed record image or iolink_ds_apply_image() rejects it with
 *     PARAM_INCONSISTENT. The DS-backed params are Application/Function/Location
 *     Tag (data_storage.c:30-34). So this firmware writes a VALID Application-Tag
 *     (0x0018) record {00 18 00 02 'D' 'S'} rather than an opaque blob, and the
 *     readback (rebuilt from device params, app-tag first) is content-checked.
 * -------------------------------------------------------------------------
 *
 * Built as a standard STM32CubeL4 project (CMSIS startup/system/linker), with
 * peripherals driven through the CMSIS register definitions — no hand-computed
 * register addresses.
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

/* --- observability: the Rust integration test's only window --- */
volatile uint8_t g_master_state = 0xFFu; /* raw iolink_master_state_t (3==OPERATE,4==ERROR) */
volatile uint8_t g_master_pd0 = 0xFFu;   /* latest PD-in byte (the device's echo) */
volatile uint8_t g_phase = 0u;           /* service-script phase */
volatile uint8_t g_isdu_ok = 0u;         /* 1 proven, 0xEE service error, 0 not reached */
volatile uint8_t g_isdu_vendor[8] = {0}; /* vendor-name bytes read over ISDU */
volatile uint8_t g_isdu_vendor_len = 0u;
volatile uint8_t g_pd_echo_ok = 0u;      /* 1 when device mirrored our PD-out */
volatile uint8_t g_event_ok = 0u;        /* 1 event details read, 0xEE on error */
volatile uint8_t g_event_code_hi = 0u;
volatile uint8_t g_event_code_lo = 0u;
volatile uint8_t g_ds_ok = 0u;           /* 1 DS write+readback round-trip, 0xEE error */
volatile uint8_t g_svc_done = 0u;        /* 1 once the whole script finished */
volatile uint8_t g_error_seen = 0u;      /* 1 once the port was observed in ERROR */
volatile uint8_t g_restart_count = 0u;   /* number of ERROR->restart recoveries */
volatile uint8_t g_diag_ck_errors = 0u;  /* low byte of diagnostics.checksum_errors */
volatile uint8_t g_diag_timeouts = 0u;   /* low byte of diagnostics.response_timeouts */
volatile uint8_t g_diag_event_pending = 0u;

/* RCC (STM32L4, RM0351 §6.4) — the simulator models clock-gating, so USART1
 * (debug, APB2) and USART2 (IO-Link PHY, APB1) are unclocked out of reset. */
static void rcc_init(void) {
    RCC->APB2ENR |= RCC_APB2ENR_USART1EN;   /* debug UART */
    RCC->APB1ENR1 |= RCC_APB1ENR1_USART2EN; /* IO-Link C/Q PHY */
    /* GPIOA carries both of them: PA2/PA3 are the USART2 C/Q pair and PA9 is
     * the debug console TX. MODER and AFR are dead until the port is clocked
     * (RM0351 6.4.17), so without this the pads never leave the GPIO block and
     * a probe on the C/Q line shows the idle latch, not the serial waveform. */
    RCC->AHB2ENR |= RCC_AHB2ENR_GPIOAEN;
}

/* The master stack schedules cycles against `now_100us`; the modeled wire is
 * the only real clock there is, so derive it from DWT->CYCCNT (4 MHz core). A
 * loop-counter clock that just adds 1 per iteration outruns modeled time, the
 * stack's deadlines become meaningless, and it rams frames at the device until
 * the device drops the link. */
#define CYCLES_PER_100US 400u

static void time_init(void) {
    CoreDebug->DEMCR |= CoreDebug_DEMCR_TRCENA_Msk;
    DWT->CYCCNT = 0u;
    DWT->CTRL |= DWT_CTRL_CYCCNTENA_Msk;
}

int main(void) {
    rcc_init();
    dbg_uart_init();
    time_init();
    dbg_puts("SVC MASTER BOOT\r\n");

    iolink_master_port_t port;
    iolink_master_config_t cfg = phy_labwired_master_config(); /* pd_out_len==1 */
    const iolink_phy_api_t *phy = phy_labwired_master_phy();

    if (iolink_master_init(&port, phy, &cfg) != 0) {
        g_master_state = 0xEEu; /* init failure sentinel */
        for (;;) {
        }
    }

    /* Valid Data-Storage image: one Application-Tag (0x0018) record carrying the
     * ASCII "DS" — [00 18][00][02]['D']['S']. See question (5) above. */
    static const uint8_t DS_IMG[6] = {0x00u, 0x18u, 0x00u, 0x02u, 0x44u, 0x53u};

    uint8_t vendor_buf[16];
    uint32_t next_tick = 0u;
    uint8_t last_state = 0xFEu;
    uint8_t last_pd = 0xFEu;
    for (;;) {
        uint32_t now = DWT->CYCCNT / CYCLES_PER_100US;

        /* One cycle-due tick per 2 ms of modeled time: startup steps are not
         * cycle-paced by the stack, so ticking every loop iteration re-sends a
         * pending DeviceOperate write until its CKS-only reply is consumed, and
         * the duplicate Type-0 frame resets the device. */
        if ((int32_t)(now - next_tick) >= 0) {
            iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, now);
            next_tick = now + 20u;
        }

        /* Response-timeout scheduling: a real master integration must tell the
         * stack when a requested reply is overdue — the CYCLE_DUE tick alone
         * never trips iolink_master_on_timeout, so without this a silent/absent
         * device is invisible (the port spins in OPERATE forever). Once the
         * cycle's response_deadline has passed while still awaiting a reply,
         * issue a RESPONSE_TIMEOUT tick so a muted device is detected, counted
         * (diagnostics.response_timeouts), and eventually drives the port to
         * ERROR after the retry budget. The stack self-gates on awaiting_response
         * so this is a no-op when a reply already arrived. */
        {
            iolink_master_timing_t t;
            if (iolink_master_get_timing(&port, &t) == 0 && t.cycle_timer_valid &&
                t.awaiting_response &&
                (int32_t)(now - t.response_deadline_100us) >= 0) {
                iolink_master_tick_at(&port, IOLINK_MASTER_TICK_RESPONSE_TIMEOUT, now);
            }
        }

        g_master_state = (uint8_t)iolink_master_get_state(&port);

        uint8_t pd[1] = {0u};
        uint8_t n = 0u;
        if (iolink_master_get_pd_in(&port, pd, sizeof(pd), &n) == 0 && n >= 1u) {
            g_master_pd0 = pd[0];
        }

        /* ---- phased service script (ISDU-family calls are polled: call each
         * loop with IDENTICAL args, advance on 0==OK / negative==error) ---- */
        switch (g_phase) {
        case 0: /* wait for OPERATE */
            if (g_master_state == 3u) {
                g_phase = 1u;
                dbg_puts("SVC PHASE 1 ISDU\r\n");
            }
            break;
        case 1: { /* ISDU read vendor name (index 0x0010) */
            uint8_t len = (uint8_t)sizeof vendor_buf;
            int r = iolink_master_read_isdu(&port, 0x0010u, 0u, vendor_buf, &len);
            if (r == 0) {
                uint8_t m = len < 8u ? len : 8u;
                for (uint8_t i = 0u; i < m; i++) {
                    g_isdu_vendor[i] = vendor_buf[i];
                }
                g_isdu_vendor_len = m;
                g_isdu_ok = 1u;
                g_phase = 2u;
                dbg_puts("SVC PHASE 2 PDOUT\r\n");
            } else if (r < 0) {
                g_isdu_ok = 0xEEu;
                g_phase = 2u;
            }
            break;
        }
        case 2: { /* PD-out echo: send 0x42, wait for the device's mirror */
            uint8_t v = 0x42u;
            (void)iolink_master_set_pd_out(&port, &v, 1u);
            if (g_master_pd0 == 0x42u) {
                g_pd_echo_ok = 1u;
                g_phase = 3u;
                dbg_puts("SVC PHASE 3 EVENT\r\n");
            }
            break;
        }
        case 3: { /* trigger the device event (0xE7) + observe event_pending */
            uint8_t v = 0xE7u;
            (void)iolink_master_set_pd_out(&port, &v, 1u);
            iolink_master_diagnostics_t d;
            if (iolink_master_get_diagnostics(&port, &d) == 0 && d.event_pending) {
                g_diag_event_pending = 1u;
                g_phase = 4u;
                dbg_puts("SVC PHASE 4 EVREAD\r\n");
            }
            break;
        }
        case 4: { /* read event details (Table 59 diagnosis readout) */
            iolink_master_event_t evs[4];
            uint8_t cnt = 0u;
            int r = iolink_master_read_event_details(&port, evs, 4u, &cnt);
            if (r == 0 && cnt >= 1u) {
                g_event_code_hi = (uint8_t)(evs[0].code >> 8);
                g_event_code_lo = (uint8_t)(evs[0].code & 0xFFu);
                g_event_ok = 1u;
                g_phase = 5u;
                dbg_puts("SVC PHASE 5 DS\r\n");
            } else if (r < 0) {
                g_event_ok = 0xEEu;
                g_phase = 5u;
            }
            break;
        }
        case 5: { /* data-storage write (valid Application-Tag record) */
            int r = iolink_master_write_data_storage(&port, DS_IMG, (uint8_t)sizeof DS_IMG);
            if (r == 0) {
                g_phase = 6u;
            } else if (r < 0) {
                g_ds_ok = 0xEEu;
                g_phase = 7u;
            }
            break;
        }
        case 6: { /* data-storage readback + content round-trip check */
            uint8_t buf[64];
            uint8_t len = (uint8_t)sizeof buf;
            int r = iolink_master_read_data_storage(&port, buf, &len);
            if (r == 0) {
                /* Readback is rebuilt from device params, Application-Tag first
                 * (data_storage.c k_ds_params order): the leading record must be
                 * the {00 18 00 02 'D' 'S'} we wrote. */
                if (len >= 6u && buf[0] == 0x00u && buf[1] == 0x18u && buf[2] == 0x00u &&
                    buf[3] == 0x02u && buf[4] == 0x44u && buf[5] == 0x53u) {
                    g_ds_ok = 1u;
                } else {
                    g_ds_ok = 0xEEu;
                }
                g_phase = 7u;
            } else if (r < 0) {
                g_ds_ok = 0xEEu;
                g_phase = 7u;
            }
            break;
        }
        case 7:
            g_svc_done = 1u;
            break;
        default:
            break;
        }

        /* ---- every loop: mirror diagnostics ---- */
        {
            iolink_master_diagnostics_t d;
            if (iolink_master_get_diagnostics(&port, &d) == 0) {
                g_diag_ck_errors = (uint8_t)d.checksum_errors;
                g_diag_timeouts = (uint8_t)d.response_timeouts;
                if (d.event_pending) {
                    g_diag_event_pending = 1u;
                }
            }
        }

        /* ---- ERROR recovery policy: count once, restart the port, re-run ---- */
        if (g_master_state == 4u) {
            g_error_seen = 1u;
            (void)iolink_master_restart(&port); /* question (1): recovers from ERROR */
            g_restart_count++;
            g_phase = 0u; /* re-run the script after the link re-negotiates */
            dbg_puts("SVC RESTART\r\n");
        }

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
