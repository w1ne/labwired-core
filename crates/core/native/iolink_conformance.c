#include "iolinki/application.h"
#include "iolinki/iolink.h"
#include "iolinki/protocol.h"
#include "iolinki_master/master.h"

#include <stddef.h>
#include <stdint.h>
#include <string.h>

#define LW_IOLM_LINK_QUEUE_CAP 128U
#define LW_IOLM_MAX_PD_LEN 32U
#define LW_IOLM_STATUS_OK 0
#define LW_IOLM_ERR_INVALID_ARG -1
#define LW_IOLM_ERR_INIT -2
#define LW_IOLM_ERR_NO_OPERATE -3
#define LW_IOLM_ERR_CYCLIC -4

typedef struct
{
    uint8_t bytes[LW_IOLM_LINK_QUEUE_CAP];
    uint8_t head;
    uint8_t len;
} lw_iolm_link_queue_t;

typedef struct
{
    int32_t master_state;
    uint8_t pd_in_len;
    uint8_t pd_out_len;
    uint8_t pd_in[LW_IOLM_MAX_PD_LEN];
    uint8_t device_observed_pd_input_len;
    uint8_t device_observed_pd_input[LW_IOLM_MAX_PD_LEN];
    uint8_t device_observed_pd_output_len;
    uint8_t device_observed_pd_output[LW_IOLM_MAX_PD_LEN];
    uint8_t cycles;
} lw_iolm_conformance_result_t;

static lw_iolm_link_queue_t g_master_to_device;
static lw_iolm_link_queue_t g_device_to_master;
static int g_wakeup_pending;
static uint8_t g_last_pd_input[LW_IOLM_MAX_PD_LEN];
static uint8_t g_last_pd_input_len;
static uint8_t g_last_pd_output[LW_IOLM_MAX_PD_LEN];
static uint8_t g_last_pd_output_len;

static void q_reset(lw_iolm_link_queue_t* q)
{
    memset(q, 0, sizeof(*q));
}

static int q_push(lw_iolm_link_queue_t* q, const uint8_t* data, size_t len)
{
    size_t i;

    if(((data == NULL) && (len > 0U)) || ((size_t)q->len + len > LW_IOLM_LINK_QUEUE_CAP))
    {
        return LW_IOLM_ERR_INVALID_ARG;
    }

    for(i = 0U; i < len; i++)
    {
        uint8_t tail = (uint8_t)((q->head + q->len) % LW_IOLM_LINK_QUEUE_CAP);
        q->bytes[tail] = data[i];
        q->len++;
    }

    return (int)len;
}

static int q_pop(lw_iolm_link_queue_t* q, uint8_t* byte)
{
    if((byte == NULL) || (q->len == 0U))
    {
        return 0;
    }

    *byte = q->bytes[q->head];
    q->head = (uint8_t)((q->head + 1U) % LW_IOLM_LINK_QUEUE_CAP);
    q->len--;
    return 1;
}

static int master_send(const uint8_t* data, size_t len)
{
    return q_push(&g_master_to_device, data, len);
}

static int master_recv(uint8_t* byte)
{
    return q_pop(&g_device_to_master, byte);
}

static int master_wake_up(void)
{
    g_wakeup_pending = 1;
    return 0;
}

static int checked_set_mode(iolink_phy_mode_t mode)
{
    (void)mode;
    return 0;
}

static int phy_noop(void)
{
    return 0;
}

static int device_send(const uint8_t* data, size_t len)
{
    return q_push(&g_device_to_master, data, len);
}

static int device_recv(uint8_t* byte)
{
    return q_pop(&g_master_to_device, byte);
}

static int device_detect_wakeup(void)
{
    int ret = g_wakeup_pending;
    g_wakeup_pending = 0;
    return ret;
}

static void device_set_mode(iolink_phy_mode_t mode)
{
    (void)mode;
}

static void device_set_baudrate(iolink_baudrate_t baudrate)
{
    (void)baudrate;
}

static void on_device_pd_input(const uint8_t* data, uint8_t len)
{
    if(len > LW_IOLM_MAX_PD_LEN)
    {
        len = LW_IOLM_MAX_PD_LEN;
    }
    memcpy(g_last_pd_input, data, len);
    g_last_pd_input_len = len;
}

static void on_device_pd_output(uint8_t* data, uint8_t len)
{
    if(len > LW_IOLM_MAX_PD_LEN)
    {
        len = LW_IOLM_MAX_PD_LEN;
    }
    memcpy(g_last_pd_output, data, len);
    g_last_pd_output_len = len;
}

static void fill_incrementing(uint8_t* data, uint8_t len, uint8_t first)
{
    uint8_t i;

    for(i = 0U; i < len; i++)
    {
        data[i] = (uint8_t)(first + i);
    }
}

static int pump_device(const uint8_t* pd, uint8_t len)
{
    uint8_t i;

    if(iolink_pd_input_update(pd, len, true) != 0)
    {
        return LW_IOLM_ERR_CYCLIC;
    }
    for(i = 0U; i < 4U; i++)
    {
        iolink_process();
    }
    return LW_IOLM_STATUS_OK;
}

static iolink_m_seq_type_t device_mseq_for_master(iolink_master_m_seq_type_t type)
{
    switch(type)
    {
    case IOLINK_MASTER_M_SEQ_TYPE_1_1:
        return IOLINK_M_SEQ_TYPE_1_1;
    case IOLINK_MASTER_M_SEQ_TYPE_1_2:
        return IOLINK_M_SEQ_TYPE_1_2;
    case IOLINK_MASTER_M_SEQ_TYPE_1_V:
        return IOLINK_M_SEQ_TYPE_1_V;
    case IOLINK_MASTER_M_SEQ_TYPE_2_1:
        return IOLINK_M_SEQ_TYPE_2_1;
    case IOLINK_MASTER_M_SEQ_TYPE_2_2:
        return IOLINK_M_SEQ_TYPE_2_2;
    case IOLINK_MASTER_M_SEQ_TYPE_2_V:
        return IOLINK_M_SEQ_TYPE_2_V;
    case IOLINK_MASTER_M_SEQ_TYPE_0:
    default:
        return IOLINK_M_SEQ_TYPE_0;
    }
}

static void reset_link(void)
{
    q_reset(&g_master_to_device);
    q_reset(&g_device_to_master);
    g_wakeup_pending = 0;
    g_last_pd_input_len = 0U;
    g_last_pd_output_len = 0U;
    memset(g_last_pd_input, 0, sizeof(g_last_pd_input));
    memset(g_last_pd_output, 0, sizeof(g_last_pd_output));
}

int lw_iolm_conformance_run_profile(uint8_t m_seq_type,
                                    uint8_t pd_in_len,
                                    uint8_t pd_out_len,
                                    uint8_t pd_value,
                                    lw_iolm_conformance_result_t* result)
{
    static const iolink_phy_api_t master_phy = {
        .send = master_send,
        .recv_byte = master_recv,
    };
    static const iolink_phy_api_t device_phy = {
        .init = phy_noop,
        .set_mode = device_set_mode,
        .set_baudrate = device_set_baudrate,
        .send = device_send,
        .recv_byte = device_recv,
        .detect_wakeup = device_detect_wakeup,
    };
    static const iolink_app_callbacks_t app_callbacks = {
        .on_pd_input = on_device_pd_input,
        .on_pd_output = on_device_pd_output,
    };
    iolink_master_port_t master;
    iolink_master_config_t master_config = {
        .port_mode = IOLINK_MASTER_PORT_MODE_IOLINK,
        .m_seq_type = (iolink_master_m_seq_type_t)m_seq_type,
        .baudrate = IOLINK_BAUDRATE_COM2,
        .min_cycle_time = 10U,
        .pd_in_len = pd_in_len,
        .pd_out_len = pd_out_len,
        .response_timeout_100us = 20U,
        .set_mode_checked = checked_set_mode,
        .prepare_tx = phy_noop,
        .prepare_rx = phy_noop,
        .wake_up = master_wake_up,
    };
    iolink_config_t device_config = {
        .m_seq_type = device_mseq_for_master((iolink_master_m_seq_type_t)m_seq_type),
        .min_cycle_time = 10U,
        .pd_in_len = pd_in_len,
        .pd_out_len = pd_out_len,
        .t_pd_us = 0U,
    };
    uint8_t device_pd[LW_IOLM_MAX_PD_LEN] = {0U};
    uint8_t master_pd_out[LW_IOLM_MAX_PD_LEN] = {0U};
    uint8_t pd_len = 0U;
    uint8_t cycle;

    if((result == NULL) || (pd_in_len > LW_IOLM_MAX_PD_LEN) ||
       (pd_out_len > LW_IOLM_MAX_PD_LEN))
    {
        return LW_IOLM_ERR_INVALID_ARG;
    }

    memset(result, 0, sizeof(*result));
    reset_link();
    fill_incrementing(device_pd, pd_in_len, pd_value);
    fill_incrementing(master_pd_out, pd_out_len, (uint8_t)(pd_value ^ 0x55U));

    iolink_app_register(&app_callbacks);
    if(iolink_init(&device_phy, &device_config) != 0)
    {
        return LW_IOLM_ERR_INIT;
    }
    iolink_set_timing_enforcement(false);
    if(iolink_master_init(&master, &master_phy, &master_config) != 0)
    {
        return LW_IOLM_ERR_INIT;
    }
    if(iolink_master_set_pd_out(&master, master_pd_out, pd_out_len) != 0)
    {
        return LW_IOLM_ERR_INIT;
    }

    for(cycle = 0U; cycle < 20U; cycle++)
    {
        if(iolink_master_tick_at(&master, IOLINK_MASTER_TICK_CYCLE_DUE, cycle) != 0)
        {
            return LW_IOLM_ERR_CYCLIC;
        }
        if(pump_device(device_pd, pd_in_len) != 0)
        {
            return LW_IOLM_ERR_CYCLIC;
        }
        (void)iolink_master_tick_at(&master, IOLINK_MASTER_TICK_NONE, (uint32_t)(cycle + 1U));

        if(iolink_master_get_state(&master) == IOLINK_MASTER_STATE_OPERATE)
        {
            if(iolink_master_tick_at(&master,
                                     IOLINK_MASTER_TICK_CYCLE_DUE,
                                     (uint32_t)(cycle + 40U)) != 0)
            {
                return LW_IOLM_ERR_CYCLIC;
            }
            if(pump_device(device_pd, pd_in_len) != 0)
            {
                return LW_IOLM_ERR_CYCLIC;
            }
            if(iolink_master_tick_at(&master,
                                     IOLINK_MASTER_TICK_NONE,
                                     (uint32_t)(cycle + 41U)) < 0)
            {
                return LW_IOLM_ERR_CYCLIC;
            }
            if(iolink_master_get_pd_in(&master,
                                       result->pd_in,
                                       sizeof(result->pd_in),
                                       &pd_len) != 0)
            {
                return LW_IOLM_ERR_CYCLIC;
            }

            result->master_state = (int32_t)iolink_master_get_state(&master);
            result->pd_in_len = pd_len;
            result->pd_out_len = pd_out_len;
            result->device_observed_pd_input_len = g_last_pd_input_len;
            result->device_observed_pd_output_len = g_last_pd_output_len;
            memcpy(result->device_observed_pd_input,
                   g_last_pd_input,
                   sizeof(result->device_observed_pd_input));
            memcpy(result->device_observed_pd_output,
                   g_last_pd_output,
                   sizeof(result->device_observed_pd_output));
            result->cycles = (uint8_t)(cycle + 1U);
            return LW_IOLM_STATUS_OK;
        }
    }

    result->master_state = (int32_t)iolink_master_get_state(&master);
    return LW_IOLM_ERR_NO_OPERATE;
}
