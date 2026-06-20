/* Spatial thermal-fingerprint + fault classification for a 24×32 °C field.
 *
 * This is the device's intellectual core. It is BLIND to the simulation's
 * scene config: every threshold below is a firmware-side #define, and the
 * COOLING_FAILURE verdict is inferred purely from the observed heating-rate
 * behaviour over time — never from `cooling_fault_at_s` (which the firmware
 * cannot see). Positioned for electrical-cabinet / cooling faults where heat
 * LEADS the failure.
 */
#ifndef THERMAL_FINGERPRINT_H
#define THERMAL_FINGERPRINT_H

#include <stdint.h>

#define TFS_ROWS 24
#define TFS_COLS 32
#define TFS_PIXELS (TFS_ROWS * TFS_COLS)

/* ── Firmware-side thresholds (NOT from the scene config) ──────────────────── */
#define TFS_WARN_C       58.0f   /* ΔT/abs warn level                          */
#define TFS_CRITICAL_C   70.0f   /* hotspot ≥ this → OVERTEMP                   */
#define TFS_EXPECTED_TAU 4.0f    /* expected thermal time constant, s          */
/* By ~3τ a healthy first-order warm-up should be ~95% settled and its heating
 * rate should have decayed to near zero. If, well past 3τ, the rate stays
 * above this floor, cooling is failing (runaway). */
#define TFS_RATE_FLOOR   0.30f   /* °C/s the rate must fall below to be STABLE  */
#define TFS_SETTLE_TIME  (3.0f * TFS_EXPECTED_TAU) /* warm-up grace, s          */
/* Localized-hotspot emergence: a pixel whose ΔT vs the field spikes by this
 * much between frames (electrical/connection hotspot). */
#define TFS_EMERGE_JUMP_C 8.0f

/* State machine. */
typedef enum {
    TFS_IDLE = 0,
    TFS_WARMUP,
    TFS_STABLE,
    TFS_FAULT
} tfs_state_t;

/* Fault classification. */
typedef enum {
    TFS_FAULT_NONE = 0,
    TFS_FAULT_OVERTEMP,
    TFS_FAULT_COOLING_FAILURE,
    TFS_FAULT_HOTSPOT_EMERGENCE
} tfs_fault_t;

/* Event-flag bits packed into the process-data frame. */
#define TFS_EV_WARN        (1u << 0)
#define TFS_EV_CRITICAL    (1u << 1)
#define TFS_EV_RATE_HIGH   (1u << 2)
#define TFS_EV_EMERGENCE   (1u << 3)
#define TFS_EV_FAULT_LATCH (1u << 4)

/* Per-frame verdict. */
typedef struct {
    float hotspot_c;   /* max °C in the field                                  */
    int hot_row;       /* hotspot location                                     */
    int hot_col;
    float ambient_c;   /* ambient estimate (edge/min)                          */
    float delta_c;     /* hotspot − ambient                                    */
    float mean_c;      /* field mean                                           */
    float rate_c_s;    /* heating rate of the hotspot, °C/s                    */
    tfs_state_t state;
    tfs_fault_t fault;
    int health;        /* 0..100                                              */
    uint16_t time_to_limit_s; /* seconds to reach critical at current rate    */
    uint16_t event_flags;
} tfs_verdict_t;

/* Opaque cross-frame state. */
typedef struct {
    int initialized;
    float prev_hot_c;
    float prev_time_s;
    float warmup_peak_rate;  /* max heating rate seen during warm-up           */
    int   rate_history_n;    /* frames observed                                */
    tfs_state_t state;
    int fault_latched;       /* once FAULT, stays FAULT                        */
} tfs_ctx_t;

void tfs_init(tfs_ctx_t *ctx);

/* Run the fingerprint on one decoded field at simulated time `time_s`.
 * `field` is TFS_PIXELS floats in °C (row-major, 32 cols). Fills `out`. */
void tfs_update(tfs_ctx_t *ctx, const float *field, float time_s,
                tfs_verdict_t *out);

/* Names for printing. */
const char *tfs_state_name(tfs_state_t s);
const char *tfs_fault_name(tfs_fault_t f);

#endif /* THERMAL_FINGERPRINT_H */
