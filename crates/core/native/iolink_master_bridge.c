#include "iolinki_master/master.h"

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#define LW_IOLM_QUEUE_CAP 128U
#define LW_IOLM_STATUS_OK 0
#define LW_IOLM_ERR_INVALID_ARG -1

typedef struct
{
    iolink_master_port_t port;
    uint8_t tx[LW_IOLM_QUEUE_CAP];
    size_t tx_len;
    uint8_t rx[LW_IOLM_QUEUE_CAP];
    size_t rx_head;
    size_t rx_len;
    uint32_t wake_count;
} lw_iolm_bridge_t;

static lw_iolm_bridge_t* g_active_bridge = NULL;

static int bridge_push_tx(lw_iolm_bridge_t* bridge, uint8_t byte)
{
    if((bridge == NULL) || (bridge->tx_len >= LW_IOLM_QUEUE_CAP))
    {
        return LW_IOLM_ERR_INVALID_ARG;
    }
    bridge->tx[bridge->tx_len++] = byte;
    return LW_IOLM_STATUS_OK;
}

static int bridge_send(const uint8_t* data, size_t len)
{
    size_t i;

    if((g_active_bridge == NULL) || ((data == NULL) && (len > 0U)))
    {
        return LW_IOLM_ERR_INVALID_ARG;
    }
    if((g_active_bridge->tx_len + len) > LW_IOLM_QUEUE_CAP)
    {
        return LW_IOLM_ERR_INVALID_ARG;
    }

    for(i = 0U; i < len; i++)
    {
        g_active_bridge->tx[g_active_bridge->tx_len++] = data[i];
    }
    return (int)len;
}

static int bridge_recv_byte(uint8_t* byte)
{
    if((g_active_bridge == NULL) || (byte == NULL))
    {
        return LW_IOLM_ERR_INVALID_ARG;
    }
    if(g_active_bridge->rx_len == 0U)
    {
        return LW_IOLM_STATUS_OK;
    }

    *byte = g_active_bridge->rx[g_active_bridge->rx_head];
    g_active_bridge->rx_head = (g_active_bridge->rx_head + 1U) % LW_IOLM_QUEUE_CAP;
    g_active_bridge->rx_len--;
    if(g_active_bridge->rx_len == 0U)
    {
        g_active_bridge->rx_head = 0U;
    }
    return 1;
}

static int bridge_noop(void)
{
    return LW_IOLM_STATUS_OK;
}

static int bridge_wake_up(void)
{
    if(g_active_bridge == NULL)
    {
        return LW_IOLM_ERR_INVALID_ARG;
    }
    g_active_bridge->wake_count++;
    return bridge_push_tx(g_active_bridge, 0x55U);
}

static int bridge_with_active(lw_iolm_bridge_t* bridge,
                              iolink_master_tick_event_t event,
                              uint32_t now_100us)
{
    int ret;

    if(bridge == NULL)
    {
        return LW_IOLM_ERR_INVALID_ARG;
    }
    g_active_bridge = bridge;
    ret = iolink_master_tick_at(&bridge->port, event, now_100us);
    g_active_bridge = NULL;
    return ret;
}

lw_iolm_bridge_t* lw_iolm_bridge_new(uint8_t m_seq_type,
                                     uint8_t pd_in_len,
                                     uint8_t pd_out_len,
                                     uint8_t min_cycle_time,
                                     uint8_t response_timeout_100us)
{
    static const iolink_phy_api_t phy = {
        .send = bridge_send,
        .recv_byte = bridge_recv_byte,
    };
    lw_iolm_bridge_t* bridge = (lw_iolm_bridge_t*)calloc(1U, sizeof(lw_iolm_bridge_t));
    int ret;

    if(bridge == NULL)
    {
        return NULL;
    }

    iolink_master_config_t config = {
        .port_mode = IOLINK_MASTER_PORT_MODE_IOLINK,
        .m_seq_type = (iolink_master_m_seq_type_t)m_seq_type,
        .baudrate = IOLINK_BAUDRATE_COM2,
        .min_cycle_time = min_cycle_time,
        .pd_in_len = pd_in_len,
        .pd_out_len = pd_out_len,
        .response_timeout_100us = response_timeout_100us,
        .prepare_tx = bridge_noop,
        .prepare_rx = bridge_noop,
        .wake_up = bridge_wake_up,
    };

    g_active_bridge = bridge;
    ret = iolink_master_init(&bridge->port, &phy, &config);
    g_active_bridge = NULL;
    if(ret != IOLINK_MASTER_STATUS_OK)
    {
        free(bridge);
        return NULL;
    }

    return bridge;
}

void lw_iolm_bridge_free(lw_iolm_bridge_t* bridge)
{
    free(bridge);
}

int lw_iolm_bridge_set_pd_out(lw_iolm_bridge_t* bridge, const uint8_t* data, uint8_t len)
{
    if(bridge == NULL)
    {
        return LW_IOLM_ERR_INVALID_ARG;
    }
    return iolink_master_set_pd_out(&bridge->port, data, len);
}

int lw_iolm_bridge_cycle_due(lw_iolm_bridge_t* bridge, uint32_t now_100us)
{
    return bridge_with_active(bridge, IOLINK_MASTER_TICK_CYCLE_DUE, now_100us);
}

int lw_iolm_bridge_tick_none(lw_iolm_bridge_t* bridge, uint32_t now_100us)
{
    return bridge_with_active(bridge, IOLINK_MASTER_TICK_NONE, now_100us);
}

int lw_iolm_bridge_feed_rx(lw_iolm_bridge_t* bridge, const uint8_t* data, size_t len)
{
    size_t tail;
    size_t i;

    if((bridge == NULL) || ((data == NULL) && (len > 0U)))
    {
        return LW_IOLM_ERR_INVALID_ARG;
    }
    if((bridge->rx_len + len) > LW_IOLM_QUEUE_CAP)
    {
        return LW_IOLM_ERR_INVALID_ARG;
    }

    for(i = 0U; i < len; i++)
    {
        tail = (bridge->rx_head + bridge->rx_len) % LW_IOLM_QUEUE_CAP;
        bridge->rx[tail] = data[i];
        bridge->rx_len++;
    }

    return LW_IOLM_STATUS_OK;
}

size_t lw_iolm_bridge_drain_tx(lw_iolm_bridge_t* bridge, uint8_t* out, size_t out_len)
{
    size_t n;

    if((bridge == NULL) || (out == NULL))
    {
        return 0U;
    }

    n = bridge->tx_len;
    if(n > out_len)
    {
        n = out_len;
    }
    if(n > 0U)
    {
        memcpy(out, bridge->tx, n);
    }
    if(n < bridge->tx_len)
    {
        memmove(bridge->tx, bridge->tx + n, bridge->tx_len - n);
    }
    bridge->tx_len -= n;

    return n;
}

int lw_iolm_bridge_state(const lw_iolm_bridge_t* bridge)
{
    if(bridge == NULL)
    {
        return IOLINK_MASTER_STATE_ERROR;
    }
    return (int)iolink_master_get_state(&bridge->port);
}

int lw_iolm_bridge_get_pd_in(const lw_iolm_bridge_t* bridge,
                             uint8_t* out,
                             uint8_t out_len,
                             uint8_t* actual_len)
{
    if(bridge == NULL)
    {
        return LW_IOLM_ERR_INVALID_ARG;
    }
    return iolink_master_get_pd_in(&bridge->port, out, out_len, actual_len);
}

uint32_t lw_iolm_bridge_wake_count(const lw_iolm_bridge_t* bridge)
{
    if(bridge == NULL)
    {
        return 0U;
    }
    return bridge->wake_count;
}
