#ifndef LABWIRED_IOLINK_MASTER_BRIDGE_H
#define LABWIRED_IOLINK_MASTER_BRIDGE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define LW_IOLM_QUEUE_CAP 256U

typedef enum {
    LW_IOLM_TICK_NONE = 0,
    LW_IOLM_TICK_CYCLE_DUE = 1,
    LW_IOLM_TICK_RESPONSE_TIMEOUT = 2
} lw_iolm_tick_event_t;

typedef struct {
    uint8_t pd_in_len;
    uint8_t pd_out_len;
    uint8_t m_seq_type;
    uint8_t min_cycle_time_100us;
    uint8_t response_timeout_100us;
    uint8_t com;
} lw_iolm_config_t;

const char* lw_iolm_backend_name(void);
size_t lw_iolm_context_size(void);
int lw_iolm_init(void* ctx, const lw_iolm_config_t* config);
int lw_iolm_tick(void* ctx, lw_iolm_tick_event_t event, uint32_t now_100us);
size_t lw_iolm_drain_tx(void* ctx, uint8_t* out, size_t out_len);
size_t lw_iolm_feed_rx(void* ctx, const uint8_t* data, size_t len);
const char* lw_iolm_state_name(void* ctx);
size_t lw_iolm_latest_pd(void* ctx, uint8_t* out, size_t out_len);

#ifdef __cplusplus
}
#endif

#endif
