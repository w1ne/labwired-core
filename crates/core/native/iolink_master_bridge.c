#include "iolink_master_bridge.h"
#include "iolinki_master/master.h"

#include <string.h>

typedef struct {
    iolink_master_port_t port;
    /*
     * `iolink_master_init` keeps the PHY by pointer (`state->phy = phy`) while
     * copying the config by value. The PHY must therefore outlive init, so it
     * is owned by the context rather than passed as a stack local.
     */
    iolink_phy_api_t phy;
    uint8_t tx_queue[LW_IOLM_QUEUE_CAP];
    size_t tx_head;
    size_t tx_len;
    uint8_t rx_queue[LW_IOLM_QUEUE_CAP];
    size_t rx_head;
    size_t rx_len;
} lw_iolm_context_t;

static lw_iolm_context_t* g_active;

const char* lw_iolm_backend_name(void)
{
    return "iolinki-master";
}

size_t lw_iolm_context_size(void)
{
    return sizeof(lw_iolm_context_t);
}

static void q_push(uint8_t* q, size_t* head, size_t* len, uint8_t byte)
{
    if (*len >= LW_IOLM_QUEUE_CAP) {
        return;
    }
    size_t tail = (*head + *len) % LW_IOLM_QUEUE_CAP;
    q[tail] = byte;
    *len += 1U;
}

static int q_pop(uint8_t* q, size_t* head, size_t* len, uint8_t* byte)
{
    if (*len == 0U) {
        return 0;
    }
    *byte = q[*head];
    *head = (*head + 1U) % LW_IOLM_QUEUE_CAP;
    *len -= 1U;
    return 1;
}

static int bridge_send(const uint8_t* data, size_t len)
{
    if ((g_active == 0) || (data == 0)) {
        return -1;
    }
    for (size_t i = 0; i < len; i++) {
        q_push(g_active->tx_queue, &g_active->tx_head, &g_active->tx_len, data[i]);
    }
    return (int) len;
}

static int bridge_recv_byte(uint8_t* byte)
{
    if ((g_active == 0) || (byte == 0)) {
        return -1;
    }
    return q_pop(g_active->rx_queue, &g_active->rx_head, &g_active->rx_len, byte);
}

static int bridge_wake_up(void)
{
    uint8_t byte = 0x55U;
    return bridge_send(&byte, 1U) == 1 ? 0 : -1;
}

static int bridge_set_mode_checked(iolink_phy_mode_t mode)
{
    (void) mode;
    return 0;
}

static int bridge_set_baudrate_checked(iolink_baudrate_t baudrate)
{
    (void) baudrate;
    return 0;
}

static int bridge_flush_rx(void) { return 0; }
static int bridge_prepare_tx(void) { return 0; }
static int bridge_prepare_rx(void) { return 0; }

int lw_iolm_init(void* ctx, const lw_iolm_config_t* config)
{
    if ((ctx == 0) || (config == 0)) {
        return -1;
    }
    lw_iolm_context_t* c = (lw_iolm_context_t*) ctx;
    memset(c, 0, sizeof(*c));

    c->phy.send = bridge_send;
    c->phy.recv_byte = bridge_recv_byte;
    iolink_master_config_t cfg = {
        .port_mode = IOLINK_MASTER_PORT_MODE_IOLINK,
        .m_seq_type = (iolink_master_m_seq_type_t) config->m_seq_type,
        .baudrate = (iolink_baudrate_t) config->com,
        .min_cycle_time = config->min_cycle_time_100us,
        .pd_in_len = config->pd_in_len,
        .pd_out_len = config->pd_out_len,
        .auto_baudrate = false,
        .response_timeout_100us = config->response_timeout_100us,
        .set_mode_checked = bridge_set_mode_checked,
        .set_baudrate_checked = bridge_set_baudrate_checked,
        .flush_rx = bridge_flush_rx,
        .prepare_tx = bridge_prepare_tx,
        .prepare_rx = bridge_prepare_rx,
        .wake_up = bridge_wake_up,
    };

    g_active = c;
    int ret = iolink_master_init(&c->port, &c->phy, &cfg);
    g_active = 0;
    return ret;
}

int lw_iolm_tick(void* ctx, lw_iolm_tick_event_t event, uint32_t now_100us)
{
    if (ctx == 0) {
        return -1;
    }
    lw_iolm_context_t* c = (lw_iolm_context_t*) ctx;
    g_active = c;
    int ret = iolink_master_tick_at(&c->port, (iolink_master_tick_event_t) event, now_100us);
    g_active = 0;
    return ret;
}

size_t lw_iolm_drain_tx(void* ctx, uint8_t* out, size_t out_len)
{
    if ((ctx == 0) || (out == 0)) {
        return 0U;
    }
    lw_iolm_context_t* c = (lw_iolm_context_t*) ctx;
    size_t n = 0U;
    while (n < out_len && q_pop(c->tx_queue, &c->tx_head, &c->tx_len, &out[n]) > 0) {
        n++;
    }
    return n;
}

size_t lw_iolm_feed_rx(void* ctx, const uint8_t* data, size_t len)
{
    if ((ctx == 0) || (data == 0)) {
        return 0U;
    }
    lw_iolm_context_t* c = (lw_iolm_context_t*) ctx;
    for (size_t i = 0; i < len; i++) {
        q_push(c->rx_queue, &c->rx_head, &c->rx_len, data[i]);
    }
    return len;
}

const char* lw_iolm_state_name(void* ctx)
{
    if (ctx == 0) {
        return "invalid";
    }
    lw_iolm_context_t* c = (lw_iolm_context_t*) ctx;
    switch (iolink_master_get_state(&c->port)) {
        case IOLINK_MASTER_STATE_INACTIVE: return "inactive";
        case IOLINK_MASTER_STATE_STARTUP: return "startup";
        case IOLINK_MASTER_STATE_PREOPERATE: return "preoperate";
        case IOLINK_MASTER_STATE_OPERATE: return "operate";
        case IOLINK_MASTER_STATE_ERROR: return "error";
        default: return "unknown";
    }
}
