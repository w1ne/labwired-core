/* Spatial thermal-fingerprint + fault classification — see fingerprint.h.
 *
 * Design notes (why it is blind to the scene):
 *   * Every threshold is a #define in fingerprint.h (warn/critical/τ/rate).
 *   * COOLING_FAILURE is detected from RATE BEHAVIOUR: a first-order warm-up
 *     settles by ~3τ and its heating rate decays toward 0. If, past the settle
 *     grace (3τ), the hotspot keeps climbing (rate stays above TFS_RATE_FLOOR),
 *     the heat is no longer being removed — cooling has failed. The firmware
 *     never reads `cooling_fault_at_s`; it only sees decoded °C and timestamps.
 *   * OVERTEMP is a pure threshold crossing (hotspot ≥ critical).
 *   * HOTSPOT_EMERGENCE is a localized ΔT spike vs the previous frame.
 */
#include "fingerprint.h"

void tfs_init(tfs_ctx_t *ctx) {
    ctx->initialized = 0;
    ctx->prev_hot_c = 0.0f;
    ctx->prev_time_s = 0.0f;
    ctx->warmup_peak_rate = 0.0f;
    ctx->rate_history_n = 0;
    ctx->state = TFS_IDLE;
    ctx->fault_latched = 0;
}

const char *tfs_state_name(tfs_state_t s) {
    switch (s) {
    case TFS_IDLE:   return "IDLE";
    case TFS_WARMUP: return "WARMUP";
    case TFS_STABLE: return "STABLE";
    case TFS_FAULT:  return "FAULT";
    default:         return "?";
    }
}

const char *tfs_fault_name(tfs_fault_t f) {
    switch (f) {
    case TFS_FAULT_NONE:              return "NONE";
    case TFS_FAULT_OVERTEMP:          return "OVERTEMP";
    case TFS_FAULT_COOLING_FAILURE:   return "COOLING_FAILURE";
    case TFS_FAULT_HOTSPOT_EMERGENCE: return "HOTSPOT_EMERGENCE";
    default:                          return "?";
    }
}

static int clampi(int v, int lo, int hi) {
    if (v < lo) return lo;
    if (v > hi) return hi;
    return v;
}

void tfs_update(tfs_ctx_t *ctx, const float *field, float time_s,
                tfs_verdict_t *out) {
    int r, c, i;
    float hot = field[0];
    int hot_idx = 0;
    float lo = field[0];
    double sum = 0.0;

    /* ── Spatial reduction: hotspot (max + argmax), ambient (min), mean. ──── */
    for (i = 0; i < TFS_PIXELS; i++) {
        float v = field[i];
        sum += (double)v;
        if (v > hot) { hot = v; hot_idx = i; }
        if (v < lo)  { lo = v; }
    }
    /* Ambient estimate: median-ish of the cool background. The min over the
     * field is a robust floor for a localized-hotspot scene; refine it with the
     * border-pixel mean (edges are furthest from the hotspot). */
    float edge_sum = 0.0f;
    int edge_n = 0;
    for (c = 0; c < TFS_COLS; c++) {
        edge_sum += field[c];                              /* top row    */
        edge_sum += field[(TFS_ROWS - 1) * TFS_COLS + c];  /* bottom row */
        edge_n += 2;
    }
    for (r = 1; r < TFS_ROWS - 1; r++) {
        edge_sum += field[r * TFS_COLS];                   /* left col  */
        edge_sum += field[r * TFS_COLS + (TFS_COLS - 1)];  /* right col */
        edge_n += 2;
    }
    float ambient = edge_sum / (float)edge_n;
    /* Guard: never let ambient exceed the field minimum estimate. */
    if (lo < ambient) ambient = lo;

    float mean = (float)(sum / (double)TFS_PIXELS);
    float delta = hot - ambient;

    out->hotspot_c = hot;
    out->hot_row = hot_idx / TFS_COLS;
    out->hot_col = hot_idx % TFS_COLS;
    out->ambient_c = ambient;
    out->delta_c = delta;
    out->mean_c = mean;

    /* ── Temporal: heating rate of the hotspot. ──────────────────────────── */
    float rate = 0.0f;
    if (ctx->initialized) {
        float dt = time_s - ctx->prev_time_s;
        if (dt > 1e-6f) {
            rate = (hot - ctx->prev_hot_c) / dt;
        }
    }
    out->rate_c_s = rate;

    uint16_t flags = 0;
    if (hot >= TFS_WARN_C)      flags |= TFS_EV_WARN;
    if (hot >= TFS_CRITICAL_C)  flags |= TFS_EV_CRITICAL;
    if (rate >= TFS_RATE_FLOOR) flags |= TFS_EV_RATE_HIGH;

    /* Localized hotspot emergence: the hotspot jumped sharply vs last frame
     * while the bulk field stayed cool (delta grew much faster than mean). */
    tfs_fault_t emergence = TFS_FAULT_NONE;
    if (ctx->initialized) {
        float hot_jump = hot - ctx->prev_hot_c;
        if (hot_jump >= TFS_EMERGE_JUMP_C && (mean - ambient) < (delta * 0.4f)) {
            flags |= TFS_EV_EMERGENCE;
            emergence = TFS_FAULT_HOTSPOT_EMERGENCE;
        }
    }

    /* ── State machine. ──────────────────────────────────────────────────── */
    tfs_fault_t fault = TFS_FAULT_NONE;

    if (ctx->fault_latched) {
        ctx->state = TFS_FAULT;
    } else if (!ctx->initialized) {
        ctx->state = TFS_IDLE;
    } else {
        switch (ctx->state) {
        case TFS_IDLE:
            if (delta > 1.0f) ctx->state = TFS_WARMUP;
            break;
        case TFS_WARMUP:
            /* A healthy warm-up settles by ~3τ: past the grace, with the rate
             * decayed below the floor and below critical → STABLE. */
            if (time_s >= TFS_SETTLE_TIME && rate < TFS_RATE_FLOOR) {
                ctx->state = TFS_STABLE;
            }
            break;
        case TFS_STABLE:
        case TFS_FAULT:
            break;
        }
    }

    /* Fault detection (overrides everything; latches). */
    if (!ctx->fault_latched) {
        if (hot >= TFS_CRITICAL_C) {
            fault = TFS_FAULT_OVERTEMP;
        } else if (time_s >= TFS_SETTLE_TIME && rate >= TFS_RATE_FLOOR) {
            /* Past the thermal time constant the rate should have decayed; it
             * has not — the hotspot is running away because cooling failed. */
            fault = TFS_FAULT_COOLING_FAILURE;
        } else if (emergence != TFS_FAULT_NONE) {
            fault = emergence;
        }
        if (fault != TFS_FAULT_NONE) {
            ctx->fault_latched = 1;
            ctx->state = TFS_FAULT;
            flags |= TFS_EV_FAULT_LATCH;
        }
    } else {
        /* Latched: keep reporting the most severe active condition. */
        if (hot >= TFS_CRITICAL_C) {
            fault = TFS_FAULT_OVERTEMP;
        } else {
            fault = TFS_FAULT_COOLING_FAILURE;
        }
    }

    out->state = ctx->state;
    out->fault = fault;

    /* ── Health score 0..100: margin to critical, penalised by heating rate. */
    float margin = (TFS_CRITICAL_C - hot) / (TFS_CRITICAL_C - TFS_WARN_C);
    if (margin > 1.0f) margin = 1.0f;       /* well below warn → full margin   */
    if (margin < 0.0f) margin = 0.0f;       /* at/over critical → none         */
    float rate_pen = rate * 8.0f;           /* °C/s → penalty points           */
    if (rate_pen < 0.0f) rate_pen = 0.0f;
    int health = (int)(margin * 100.0f - rate_pen + 0.5f);
    if (hot >= TFS_CRITICAL_C) health = 0;  /* OVERTEMP pins health to 0       */
    health = clampi(health, 0, 100);
    out->health = health;

    /* ── Time-to-limit: seconds to reach critical at the current rate. ────── */
    if (rate > 0.05f && hot < TFS_CRITICAL_C) {
        float ttl = (TFS_CRITICAL_C - hot) / rate;
        if (ttl > 65535.0f) ttl = 65535.0f;
        out->time_to_limit_s = (uint16_t)(ttl + 0.5f);
    } else {
        out->time_to_limit_s = 0xFFFFu; /* not approaching the limit          */
    }
    out->event_flags = flags;

    /* Roll temporal state. */
    if (ctx->state == TFS_WARMUP && rate > ctx->warmup_peak_rate) {
        ctx->warmup_peak_rate = rate;
    }
    ctx->prev_hot_c = hot;
    ctx->prev_time_s = time_s;
    ctx->rate_history_n++;
    ctx->initialized = 1;
}
