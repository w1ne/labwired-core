#include <stddef.h>
#include <stdint.h>

#include "uds/uds_core.h"
#include "uds/uds_dtc.h"
#include "uds/uds_dtc_store.h"
#include "uds_ecu_app.h"

#define REG32(addr) (*(volatile uint32_t *) (addr))
#define ECU_SESSION_EXTENDED 0x03u /* UDS_SESSION_ID_EXTENDED (internal id) */

/* Size of the large calibration block reported by DID 0xF1A0. Chosen > 512 to
 * exercise the udslib v2.0.0 snapshot-vs-send-under-lock TX split: the positive
 * response (62 F1 A0 + 600 payload bytes = 603 bytes) is far larger than any
 * single CAN/CAN-FD frame, so the ECU must reassemble it over multi-frame
 * ISO-TP. The board tx_buffer and the ISO-TP SDU buffer must both exceed this. */
#define ECU_BIGBLOB_DID 0xF1A0u
#define ECU_BIGBLOB_SIZE 600u

/* Writable scratch DID 0x0123 (read+write, EXTENDED session only). */
static uint8_t g_scratch[4];
/* IO-controlled point 0xA001 ("test lamp") for InputOutputControl 0x2F. */
static uint8_t g_lamp[1];

/* Dynamic read callback for the >512-byte calibration DID 0xF1A0. Fills a
 * deterministic pattern (byte i = i & 0xFF) so the tester can assert it. */
static int read_bigblob(struct uds_ctx *ctx, uint16_t did, uint8_t *buf, uint16_t max_len)
{
    (void) ctx;
    if (did != ECU_BIGBLOB_DID || max_len < ECU_BIGBLOB_SIZE) {
        return -0x31; /* requestOutOfRange */
    }
    for (uint16_t i = 0u; i < ECU_BIGBLOB_SIZE; ++i) {
        buf[i] = (uint8_t) (i & 0xFFu);
    }
    return (int) ECU_BIGBLOB_SIZE;
}

/* DID table. The 0xF190 VIN storage pointer is set in fill_config (per board);
 * the table is mutable for that reason. */
static uds_did_entry_t g_dids[] = {
    {0xF190u, 17u, 0u, 0u, NULL, NULL, NULL},
    {0x0123u, 4u, UDS_SESSION_EXTENDED, 0u, NULL, NULL, g_scratch},
    {0xA001u, 1u, 0u, 0u, NULL, NULL, g_lamp},
    {ECU_BIGBLOB_DID, ECU_BIGBLOB_SIZE, 0u, 0u, read_bigblob, NULL, NULL},
};

/* Reference DTC store, seeded with one failing DTC (0x123456). */
static uds_dtc_record_t g_dtc_backing[4];
static uds_dtc_store_t g_dtc_store;

static int security_seed(struct uds_ctx *ctx, uint8_t level, uint8_t *seed, uint16_t max_len)
{
    (void) ctx;
    (void) level;
    (void) max_len;
    uds_ecu_app_log("UDS_SEED_SERVED\n");
    seed[0] = 0xDE;
    seed[1] = 0xAD;
    seed[2] = 0xBE;
    seed[3] = 0xEF;
    return 4;
}

/* fn_reset hook: faithful CMSIS NVIC_SystemReset via AIRCR (works on M3 + M33).
 * udslib calls this only AFTER the 0x11 positive response (51 01) is on the
 * transport, so the reply is on the bus before SYSRESETREQ latches. */
static void ecu_reset(uds_ctx_t *ctx, uint8_t type)
{
    (void) ctx;
    (void) type;
    __asm volatile("dsb 0xF" ::: "memory");
    REG32(0xE000ED0Cu) = (0x05FAu << 16) | (1u << 2); /* AIRCR: VECTKEY | SYSRESETREQ */
    __asm volatile("dsb 0xF" ::: "memory");
    for (;;) {
    }
}

/* fn_routine_control: routine 0x0203, startRoutine in EXTENDED session only. */
static int ecu_routine(uds_ctx_t *ctx, uint8_t type, uint16_t id, const uint8_t *data,
                       uint16_t len, uint8_t *out, uint16_t max)
{
    (void) data;
    (void) len;
    (void) max;
    if (id != 0x0203u) {
        return -0x31; /* requestOutOfRange */
    }
    if (ctx->session.active != ECU_SESSION_EXTENDED) {
        return -0x31; /* requestOutOfRange: routine requires extended session */
    }
    if (type == 0x01u) { /* startRoutine */
        out[0] = 0x00u;  /* routine status: OK */
        return 1;
    }
    return -0x31; /* requestOutOfRange: unsupported routine control type */
}

/* fn_io_control: IO point 0xA001 (test lamp) — store and echo state. */
static int ecu_io(uds_ctx_t *ctx, uint16_t id, uint8_t type, const uint8_t *data, uint16_t len,
                  uint8_t *out, uint16_t max)
{
    (void) ctx;
    (void) type;
    (void) max;
    if (id != 0xA001u) {
        return -0x31; /* requestOutOfRange */
    }
    if (len >= 1u) {
        g_lamp[0] = data[0];
    }
    out[0] = g_lamp[0];
    return 1;
}

/* fn_comm_control: accept the requested communication mode. */
static int ecu_comm(uds_ctx_t *ctx, uint8_t ctrl_type, uint8_t comm_type, uint16_t node_id)
{
    (void) ctx;
    (void) ctrl_type;
    (void) comm_type;
    (void) node_id;
    return UDS_OK;
}

void uds_ecu_app_fill_config(uds_config_t *cfg, const char *vin)
{
    g_dids[0].storage = (void *) vin; /* 17-byte VIN reported by 0xF190 */

    uds_dtc_store_init(&g_dtc_store, g_dtc_backing, 4u, 40u);
    uds_dtc_store_register(&g_dtc_store, 0x123456u, UDS_DTC_SEVERITY_CHECK_IMMEDIATELY, 0x10u,
                           UDS_DTC_FGID_EMISSIONS);
    uds_dtc_store_report_test(&g_dtc_store, 0x123456u, true); /* set testFailed status */

    cfg->did_table.entries = g_dids;
    cfg->did_table.count = (uint16_t) (sizeof(g_dids) / sizeof(g_dids[0]));
    cfg->app_data = &g_dtc_store;
    cfg->fn_dtc_list = uds_dtc_store_list_cb;
    cfg->fn_dtc_clear = uds_dtc_store_clear_cb;
    cfg->fn_routine_control = ecu_routine;
    cfg->fn_io_control = ecu_io;
    cfg->fn_comm_control = ecu_comm;
    cfg->fn_security_seed = security_seed;
    cfg->fn_reset = ecu_reset;

    /* Use case 3: configurable S3 server-side session timeout (udslib v2.0.0
     * uds_config_t.s3_ms). 0 would fall back to UDS_S3_TIMEOUT_MS (5000 ms);
     * pin it explicitly so the non-default session timeout is exercised. The
     * board sets fn_tx_complete / reset_tx_wait_ms (use case 2) itself, since
     * the TX-drain poll is peripheral-specific (bxCAN TSR vs FDCAN TXBRP). */
    cfg->s3_ms = 4000u;
}
