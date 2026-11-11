/* ESP32-C3 thermal-fingerprint firmware — the device's intellectual core.
 *
 * Boots on a simulated ESP32-C3 (RISC-V), reads a Melexis MLX90640 32×24
 * far-IR array over the real C3 I²C0 controller, decodes per-pixel °C with the
 * UNMODIFIED vendored Melexis driver running ON-TARGET (riscv32), computes a
 * spatial thermal-fingerprint + fault classification, and prints a per-frame
 * verdict over UART0.
 *
 * The classifier is BLIND to the simulation's scene config: thresholds are
 * firmware #defines (fingerprint.h) and the COOLING_FAILURE verdict is inferred
 * from heating-rate behaviour, never from `cooling_fault_at_s`.
 *
 * Per-frame human line:
 *   TFS t_ms=<time> state=<S> hot=<°C>@(r,c) dT=<°C> rate=<°C/s> health=<n> fault=<NAME> PD=<18 hex>
 *
 * The 9-byte (18-hex) IO-Link process-data frame:
 *   [int16 temp_x100][int16 heatrate_x100][u8 state][u8 health]
 *   [u16 time_to_limit_s][u8 fault<<4 | event_flags] (big-endian on the wire)
 */
#include <stdint.h>
#include <string.h>
#include "MLX90640_API.h"
#include "MLX90640_I2C_Driver.h"
#include "c3_uart.h"
#include "fingerprint.h"
#include "iolinki/iolink.h"
#include "iolinki/application.h"
#include "iolinki/device.h"
#include "iolinki/events.h"
#include "phy_c3_iolink.h"
#include "c3_time.h"

#define MLX_ADDR 0x33

/* Reentrant iolinki device context + config. File-scope statics: the helpers
 * below service the link, and iolink_device_init() keeps a pointer to the
 * config, so both must outlive main()'s init block. */
static iolink_device_ctx_t g_iol_device;
static iolink_device_config_t g_iol_cfg;
static int g_iol_ready;

/* Also pump the link after publication. Sensor wait callbacks continue this
 * service between 2 Hz conversions; the master gap is expressed in native
 * UART peer polls, not in CPU instructions. */
#define IOLINK_SERVICE_ITERS 2000u
/* Bound the immediate service work after publication. The timer-backed sensor
 * wait continues servicing UART1 until the next conversion. */
#define IOLINK_SETTLE_PUMPS 60u

/* The sensor samples at 2 Hz. Timestamp each acquired subpage with SYSTIMER;
 * a STATUS acknowledgment does not advance either device time or the scene. */
#define TFS_FRAME_COUNT 52

/* Static scratch for the driver (kept out of the stack; ~6 KB of params). */
static paramsMLX90640 g_params;
static uint16_t g_eeprom[832];
static uint16_t g_frame[834];
static float g_field[768];

/* Pack the verdict into the 9-byte IO-Link process-data frame:
 *   [int16 temp_x100][int16 heatrate_x100][u8 state][u8 health]
 *   [u16 time_to_limit_s][u8 fault<<4 | event_flags] (big-endian on the wire)
 * The last byte carries the device's fault classification (tfs_fault_t) in the
 * high nibble and the four low event-flag bits in the low nibble. The latched
 * fault is also represented by state=FAULT; its fifth flag bit is not packed. */
static void pack_pd(const tfs_verdict_t *v, uint8_t pd[9]) {
    int16_t temp_x100 = (int16_t)(v->hotspot_c * 100.0f);
    int16_t rate_x100 = (int16_t)(v->rate_c_s * 100.0f);
    pd[0] = (uint8_t)((uint16_t)temp_x100 >> 8);
    pd[1] = (uint8_t)((uint16_t)temp_x100 & 0xFF);
    pd[2] = (uint8_t)((uint16_t)rate_x100 >> 8);
    pd[3] = (uint8_t)((uint16_t)rate_x100 & 0xFF);
    pd[4] = (uint8_t)v->state;
    pd[5] = (uint8_t)v->health;
    pd[6] = (uint8_t)(v->time_to_limit_s >> 8);
    pd[7] = (uint8_t)(v->time_to_limit_s & 0xFF);
    pd[8] = (uint8_t)(((uint8_t)v->fault << 4) | (v->event_flags & 0x0Fu));
}

/* Print the 9-byte process-data frame as hex (the human/demo readout). */
static void print_pd(const tfs_verdict_t *v) {
    uint8_t pd[9];
    pack_pd(v, pd);
    for (int i = 0; i < 9; i++) {
        uart_puthex(pd[i], 2);
    }
}

static void print_verdict(const tfs_verdict_t *v, uint32_t time_ms) {
    uart_puts("TFS t_ms=");
    uart_puti((int32_t)time_ms);
    uart_puts(" state=");
    uart_puts(tfs_state_name(v->state));
    uart_puts(" hot=");
    uart_putfix2((int32_t)(v->hotspot_c * 100.0f));
    uart_puts("@(");
    uart_puti(v->hot_row);
    uart_putc(',');
    uart_puti(v->hot_col);
    uart_puts(") dT=");
    uart_putfix2((int32_t)(v->delta_c * 100.0f));
    uart_puts(" rate=");
    uart_putfix2((int32_t)(v->rate_c_s * 100.0f));
    uart_puts(" health=");
    uart_puti(v->health);
    uart_puts(" fault=");
    uart_puts(tfs_fault_name(v->fault));
    uart_puts(" PD=");
    print_pd(v);
    uart_puts("\r\n");
    /* Finish this console line before the peer can log the new PD verdict. */
    uart_flush();
}

/* Trace IO-Link DLL state transitions over the debug console (mirrors the
 * iolink-dido device gate): prints `STATE=<hex>` on change and flags OPERATE (0x04)
 * so the device-side test can confirm the link reached OPERATE. */
static iolink_dll_state_t g_last_iol_state = (iolink_dll_state_t)0xFF;
static void trace_iolink_state(void) {
    iolink_dll_state_t s = iolink_device_get_state(&g_iol_device);
    if (s != g_last_iol_state) {
        g_last_iol_state = s;
        uart_puts("STATE=");
        uart_puthex((uint8_t)s, 2);
        if (s == IOLINK_DLL_STATE_OPERATE) {
            uart_puts(" OPERATE");
        }
        uart_puts("\r\n");
    }
}

/* Called by the I2C platform hook while the sensor has no fresh subpage.
 * Continue serving the independent UART link, then wait on a timer. */
void thermal_wait_for_sensor(void) {
    if (g_iol_ready) {
        iolink_device_process(&g_iol_device);
        trace_iolink_state();
    }
    c3_sleep_us(1000u);
}

/* Service the byte-stream IO-Link peer. Returns after `iters` pumps, or once
 * OPERATE has persisted for `settle` pumps. Actual reads continue in the sensor
 * wait; this bounded immediate pump does not promise a completed cyclic read. */
static void iolink_service(uint32_t iters, uint32_t settle) {
    uint32_t operate_run = 0;
    for (uint32_t i = 0; i < iters; i++) {
        iolink_device_process(&g_iol_device);
        trace_iolink_state();
        if (iolink_device_get_state(&g_iol_device) == IOLINK_DLL_STATE_OPERATE) {
            if (++operate_run >= settle) {
                return;
            }
        } else {
            operate_run = 0;
        }
    }
}

int main(void) {
    c3_time_init();
    uart_puts("TFS BOOT\r\n");
    uart_puts("ESP32-C3 MLX90640 thermal-fingerprint\r\n");

    MLX90640_I2CInit();

    /* Bring up the sensor: dump EEPROM + extract calibration (real driver). */
    int err = MLX90640_DumpEE(MLX_ADDR, g_eeprom);
    if (err != 0) {
        uart_puts("MLX90640 DUMPEE FAIL err=");
        uart_puti(err);
        uart_puts("\r\n");
        for (;;) {
        }
    }
    err = MLX90640_ExtractParameters(g_eeprom, &g_params);
    if (err != 0) {
        uart_puts("MLX90640 EXTRACT FAIL err=");
        uart_puti(err);
        uart_puts("\r\n");
        /* Continue: ExtractParameters can flag bad/outlier pixels but still
         * yields a usable param set for our linearized calibration. */
    }

    /* Program 2 Hz chess mode. SYSTIMER supplies the acquired-frame timestamp. */
    MLX90640_SetRefreshRate(MLX_ADDR, 0x02);
    MLX90640_SetChessMode(MLX_ADDR);

    uart_puts("MLX90640 READY (real Melexis driver on riscv32)\r\n");

    /* ── Bring up the iolinki IO-Link DEVICE stack on UART1 (the C/Q line) ──
     * The 9-byte thermal verdict is published as IO-Link process data; a native
     * master attached to uart1 cyclically reads it. Reentrant API: the stack
     * lives in an explicit device context (g_iol_device) instead of a singleton.
     * memset the config first (the iolink-dido lesson: a designated-init can
     * leave t_pd_us garbage and arm a bogus power-on delay). TYPE_2_V carries
     * the 9-byte PD plus one OD byte, including typed diagnosis requests while
     * OPERATE. The legacy Type1 demo peer used Type0 diagnosis requests which
     * the current device correctly rejects in OPERATE. */
    memset(&g_iol_device, 0, sizeof(g_iol_device));
    memset(&g_iol_cfg, 0, sizeof(g_iol_cfg));
    g_iol_cfg.phy = *iolink_phy_c3_get();
    g_iol_cfg.stack.m_seq_type = IOLINK_M_SEQ_TYPE_2_V;
    g_iol_cfg.stack.min_cycle_time = 0;
    g_iol_cfg.stack.pd_in_len = 9;
    g_iol_cfg.stack.pd_out_len = 0;
    g_iol_cfg.stack.od_len = 1;
    g_iol_cfg.stack.t_pd_us = 0;
    if (iolink_device_init(&g_iol_device, &g_iol_cfg) != 0) {
        uart_puts("IOLINK INIT FAIL\r\n");
        for (;;) {
        }
    }
    /* This fixture exercises the DLL over the native UART byte stream. Its
     * stack timing enforcement is disabled; physical C/Q timing is not tested. */
    iolink_device_set_timing_enforcement(&g_iol_device, false);
    g_iol_ready = 1;
    iolink_events_ctx_t *events = iolink_device_get_events_ctx(&g_iol_device);
    uart_puts("IOLINK INIT OK\r\n");

    tfs_ctx_t ctx;
    tfs_init(&ctx);

    float ta = 25.0f;
    const int max_frames = TFS_FRAME_COUNT; /* 26 seconds at 2 Hz */
    int event_raised = 0;     /* raise a diagnostic for the first fault */
    int completed_frames = 0;
    int fault_frames = 0;

    /* Publish initial zeroed PD and begin the wake-up → OPERATE handshake.
     * Sensor waits keep servicing the link before the first real verdict. */
    {
        uint8_t pd0[9];
        memset(pd0, 0, sizeof(pd0));
        iolink_device_pd_input_update(&g_iol_device, pd0, 9, true);
        /* Pump until OPERATE is reached (then a short settle), capped. */
        iolink_service(IOLINK_SERVICE_ITERS, IOLINK_SETTLE_PUMPS);
    }

    for (int frame = 0; frame < max_frames; frame++) {
        err = MLX90640_GetFrameData(MLX_ADDR, g_frame);
        if (err < 0) {
            uart_puts("TFS FRAME ERR=");
            uart_puti(err);
            uart_puts("\r\n");
            continue;
        }

        uint64_t sample_us = c3_time_us();
        /* Real driver: ambient + per-pixel temperature reconstruction. */
        ta = MLX90640_GetTa(g_frame, &g_params);
        MLX90640_CalculateTo(g_frame, &g_params, 1.0f /* ε */, ta, g_field);

        tfs_verdict_t v;
        tfs_update(&ctx, g_field, (float)sample_us / 1000000.0f, &v);
        print_verdict(&v, (uint32_t)(sample_us / 1000u));
        completed_frames++;
        if (v.fault != TFS_FAULT_NONE) {
            fault_frames++;
        }

        /* Publish the verdict as IO-Link process data the master reads cyclically. */
        uint8_t pd[9];
        pack_pd(&v, pd);
        iolink_device_pd_input_update(&g_iol_device, pd, 9, true);

        /* Raise one diagnostic event for the first fault. OVERTEMP uses the
         * standardized temperature-overload Error; rate/shape diagnoses use
         * application-fault Error. The exact classification remains in PD.
         * The DLL sets the operate-status EVENT bit while the event is pending. */
        if (!event_raised && v.fault != TFS_FAULT_NONE) {
            uint16_t code = (v.fault == TFS_FAULT_OVERTEMP)
                                ? IOLINK_EVENTCODE_TEMPERATURE_OVERLOAD
                                : IOLINK_EVENTCODE_APPLICATION_FAULT;
            iolink_event_trigger(events, code, IOLINK_EVENT_TYPE_ERROR);
            event_raised = 1;
            uart_puts("IOLINK EVENT TRIGGERED code=");
            uart_puthex(code, 4);
            uart_puts("\r\n");
        }

        /* Service the link now and throughout the next sensor wait. */
        iolink_service(IOLINK_SERVICE_ITERS, IOLINK_SETTLE_PUMPS);
    }

    /* The capture merges UART0 console and peer diagnostics. Finish the last
     * UART1 reply before printing completion, or its MASTER log can arrive
     * between the console's CR and LF and split the completion record. */
    iolink_phy_c3_flush_tx();
    uart_puts("TFS DONE frames=");
    uart_puti(completed_frames);
    uart_puts(" faults=");
    uart_puti(fault_frames);
    uart_puts("\r\n");
    uart_flush();
    for (;;) {
    }
}
