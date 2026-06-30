#include "iolinki/application.h"
#include "iolinki/device.h"
#include "iolinki/protocol.h"
#include "iolinki_master/master.h"

#include <stddef.h>
#include <stdint.h>
#include <string.h>

#define LW_IOLM_LINK_QUEUE_CAP 128U
#define LW_IOLM_MAX_PD_LEN 32U
#define LW_IOLM_MAX_PORTS 4U
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

typedef struct
{
    iolink_master_port_t master;
    iolink_phy_api_t master_phy;
    iolink_master_config_t master_config;
    iolink_device_ctx_t device;
    iolink_device_config_t device_config;
    iolink_phy_api_t device_phy;
    iolink_app_callbacks_t app_callbacks;
    lw_iolm_link_queue_t master_to_device;
    lw_iolm_link_queue_t device_to_master;
    int wakeup_pending;
    uint8_t device_pd[LW_IOLM_MAX_PD_LEN];
    uint8_t master_pd_out[LW_IOLM_MAX_PD_LEN];
    uint8_t last_pd_input[LW_IOLM_MAX_PD_LEN];
    uint8_t last_pd_input_len;
    uint8_t last_pd_output[LW_IOLM_MAX_PD_LEN];
    uint8_t last_pd_output_len;
} lw_iolm_port_fixture_t;

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

static int master_send(void* user, const uint8_t* data, size_t len)
{
    lw_iolm_port_fixture_t* port = (lw_iolm_port_fixture_t*)user;
    return (port != NULL) ? q_push(&port->master_to_device, data, len) : LW_IOLM_ERR_INVALID_ARG;
}

static int master_recv(void* user, uint8_t* byte)
{
    lw_iolm_port_fixture_t* port = (lw_iolm_port_fixture_t*)user;
    return (port != NULL) ? q_pop(&port->device_to_master, byte) : LW_IOLM_ERR_INVALID_ARG;
}

static int master_wake_up_for(lw_iolm_port_fixture_t* port)
{
    if(port == NULL)
    {
        return LW_IOLM_ERR_INVALID_ARG;
    }
    port->wakeup_pending = 1;
    return LW_IOLM_STATUS_OK;
}

static lw_iolm_port_fixture_t* g_active_master_config_port;

static int master_wake_up(void)
{
    return master_wake_up_for(g_active_master_config_port);
}

static int checked_set_mode(iolink_phy_mode_t mode)
{
    (void)mode;
    return 0;
}

static int phy_noop(void* user)
{
    (void)user;
    return 0;
}

static int config_noop(void)
{
    return 0;
}

static int device_send(void* user, const uint8_t* data, size_t len)
{
    lw_iolm_port_fixture_t* port = (lw_iolm_port_fixture_t*)user;
    return (port != NULL) ? q_push(&port->device_to_master, data, len) : LW_IOLM_ERR_INVALID_ARG;
}

static int device_recv(void* user, uint8_t* byte)
{
    lw_iolm_port_fixture_t* port = (lw_iolm_port_fixture_t*)user;
    return (port != NULL) ? q_pop(&port->master_to_device, byte) : LW_IOLM_ERR_INVALID_ARG;
}

static int device_detect_wakeup(void* user)
{
    lw_iolm_port_fixture_t* port = (lw_iolm_port_fixture_t*)user;
    int ret;

    if(port == NULL)
    {
        return 0;
    }
    ret = port->wakeup_pending;
    port->wakeup_pending = 0;
    return ret;
}

static void device_set_mode(void* user, iolink_phy_mode_t mode)
{
    (void)user;
    (void)mode;
}

static void device_set_baudrate(void* user, iolink_baudrate_t baudrate)
{
    (void)user;
    (void)baudrate;
}

static void on_device_pd_input(const uint8_t* data, uint8_t len)
{
    lw_iolm_port_fixture_t* port = g_active_master_config_port;

    if(port == NULL)
    {
        return;
    }
    if(len > LW_IOLM_MAX_PD_LEN)
    {
        len = LW_IOLM_MAX_PD_LEN;
    }
    memcpy(port->last_pd_input, data, len);
    port->last_pd_input_len = len;
}

static void on_device_pd_output(uint8_t* data, uint8_t len)
{
    lw_iolm_port_fixture_t* port = g_active_master_config_port;

    if(port == NULL)
    {
        return;
    }
    if(len > LW_IOLM_MAX_PD_LEN)
    {
        len = LW_IOLM_MAX_PD_LEN;
    }
    memcpy(port->last_pd_output, data, len);
    port->last_pd_output_len = len;
}

static void fill_incrementing(uint8_t* data, uint8_t len, uint8_t first)
{
    uint8_t i;

    for(i = 0U; i < len; i++)
    {
        data[i] = (uint8_t)(first + i);
    }
}

static int pump_device(lw_iolm_port_fixture_t* port)
{
    uint8_t i;

    if(iolink_device_pd_input_update(&port->device,
                                     port->device_pd,
                                     port->device_config.stack.pd_in_len,
                                     true) != 0)
    {
        return LW_IOLM_ERR_CYCLIC;
    }
    for(i = 0U; i < 4U; i++)
    {
        g_active_master_config_port = port;
        iolink_device_process(&port->device);
        g_active_master_config_port = NULL;
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

static int init_fixture(lw_iolm_port_fixture_t* port,
                        uint8_t m_seq_type,
                        uint8_t pd_in_len,
                        uint8_t pd_out_len,
                        uint8_t pd_value)
{
    if((port == NULL) || (pd_in_len > LW_IOLM_MAX_PD_LEN) || (pd_out_len > LW_IOLM_MAX_PD_LEN))
    {
        return LW_IOLM_ERR_INVALID_ARG;
    }

    memset(port, 0, sizeof(*port));
    q_reset(&port->master_to_device);
    q_reset(&port->device_to_master);
    fill_incrementing(port->device_pd, pd_in_len, pd_value);
    fill_incrementing(port->master_pd_out, pd_out_len, (uint8_t)(pd_value ^ 0x55U));

    port->master_phy.user = port;
    port->master_phy.send = master_send;
    port->master_phy.recv_byte = master_recv;
    port->master_config.port_mode = IOLINK_MASTER_PORT_MODE_IOLINK;
    port->master_config.m_seq_type = (iolink_master_m_seq_type_t)m_seq_type;
    port->master_config.baudrate = IOLINK_BAUDRATE_COM2;
    port->master_config.min_cycle_time = 10U;
    port->master_config.pd_in_len = pd_in_len;
    port->master_config.pd_out_len = pd_out_len;
    port->master_config.response_timeout_100us = 20U;
    port->master_config.set_mode_checked = checked_set_mode;
    port->master_config.prepare_tx = config_noop;
    port->master_config.prepare_rx = config_noop;
    port->master_config.wake_up = master_wake_up;

    port->device_phy.user = port;
    port->device_phy.init = phy_noop;
    port->device_phy.set_mode = device_set_mode;
    port->device_phy.set_baudrate = device_set_baudrate;
    port->device_phy.send = device_send;
    port->device_phy.recv_byte = device_recv;
    port->device_phy.detect_wakeup = device_detect_wakeup;
    port->app_callbacks.on_pd_input = on_device_pd_input;
    port->app_callbacks.on_pd_output = on_device_pd_output;
    port->device_config.phy = port->device_phy;
    port->device_config.stack.m_seq_type =
        device_mseq_for_master((iolink_master_m_seq_type_t)m_seq_type);
    port->device_config.stack.min_cycle_time = 10U;
    port->device_config.stack.pd_in_len = pd_in_len;
    port->device_config.stack.pd_out_len = pd_out_len;
    port->device_config.stack.t_pd_us = 0U;
    port->device_config.app_callbacks = &port->app_callbacks;

    if(iolink_device_init(&port->device, &port->device_config) != 0)
    {
        return LW_IOLM_ERR_INIT;
    }
    iolink_device_set_timing_enforcement(&port->device, false);

    g_active_master_config_port = port;
    if(iolink_master_init(&port->master, &port->master_phy, &port->master_config) != 0)
    {
        g_active_master_config_port = NULL;
        return LW_IOLM_ERR_INIT;
    }
    g_active_master_config_port = NULL;
    if(iolink_master_set_pd_out(&port->master, port->master_pd_out, pd_out_len) != 0)
    {
        return LW_IOLM_ERR_INIT;
    }

    return LW_IOLM_STATUS_OK;
}

static int tick_fixture(lw_iolm_port_fixture_t* port, uint8_t cycle, lw_iolm_conformance_result_t* result)
{
    uint8_t pd_len = 0U;

    g_active_master_config_port = port;
    if(iolink_master_tick_at(&port->master, IOLINK_MASTER_TICK_CYCLE_DUE, cycle) != 0)
    {
        g_active_master_config_port = NULL;
        return LW_IOLM_ERR_CYCLIC;
    }
    g_active_master_config_port = NULL;
    if(pump_device(port) != 0)
    {
        return LW_IOLM_ERR_CYCLIC;
    }

    g_active_master_config_port = port;
    (void)iolink_master_tick_at(&port->master, IOLINK_MASTER_TICK_NONE, (uint32_t)(cycle + 1U));
    g_active_master_config_port = NULL;

    if(iolink_master_get_state(&port->master) == IOLINK_MASTER_STATE_OPERATE)
    {
        g_active_master_config_port = port;
        if(iolink_master_tick_at(&port->master,
                                 IOLINK_MASTER_TICK_CYCLE_DUE,
                                 (uint32_t)(cycle + 40U)) != 0)
        {
            g_active_master_config_port = NULL;
            return LW_IOLM_ERR_CYCLIC;
        }
        g_active_master_config_port = NULL;
        if(pump_device(port) != 0)
        {
            return LW_IOLM_ERR_CYCLIC;
        }
        g_active_master_config_port = port;
        if(iolink_master_tick_at(&port->master,
                                 IOLINK_MASTER_TICK_NONE,
                                 (uint32_t)(cycle + 41U)) < 0)
        {
            g_active_master_config_port = NULL;
            return LW_IOLM_ERR_CYCLIC;
        }
        g_active_master_config_port = NULL;
        if(iolink_master_get_pd_in(&port->master, result->pd_in, sizeof(result->pd_in), &pd_len) !=
           0)
        {
            return LW_IOLM_ERR_CYCLIC;
        }

        result->master_state = (int32_t)iolink_master_get_state(&port->master);
        result->pd_in_len = pd_len;
        result->pd_out_len = port->device_config.stack.pd_out_len;
        result->device_observed_pd_input_len = port->last_pd_input_len;
        result->device_observed_pd_output_len = port->last_pd_output_len;
        memcpy(result->device_observed_pd_input,
               port->last_pd_input,
               sizeof(result->device_observed_pd_input));
        memcpy(result->device_observed_pd_output,
               port->last_pd_output,
               sizeof(result->device_observed_pd_output));
        result->cycles = (uint8_t)(cycle + 1U);
    }

    return LW_IOLM_STATUS_OK;
}

static int drive_fixture_to_operate(lw_iolm_port_fixture_t* port, uint8_t* cycle)
{
    lw_iolm_conformance_result_t result;
    uint8_t i;
    int ret;

    memset(&result, 0, sizeof(result));
    for(i = 0U; i < 20U; i++)
    {
        ret = tick_fixture(port, *cycle, &result);
        (*cycle)++;
        if(ret != LW_IOLM_STATUS_OK)
        {
            return ret;
        }
        if(result.master_state == IOLINK_MASTER_STATE_OPERATE)
        {
            return LW_IOLM_STATUS_OK;
        }
    }

    return LW_IOLM_ERR_NO_OPERATE;
}

static int drive_fixture_write_isdu(lw_iolm_port_fixture_t* port,
                                    uint16_t index,
                                    uint8_t subindex,
                                    const uint8_t* data,
                                    uint8_t len,
                                    uint8_t* cycle)
{
    lw_iolm_conformance_result_t result;
    uint8_t i;
    int ret = iolink_master_write_isdu(&port->master, index, subindex, data, len);

    if(ret != IOLINK_MASTER_STATUS_PENDING)
    {
        return ret;
    }

    for(i = 0U; i < 80U; i++)
    {
        memset(&result, 0, sizeof(result));
        ret = tick_fixture(port, *cycle, &result);
        (*cycle)++;
        if(ret != LW_IOLM_STATUS_OK)
        {
            return ret;
        }
        ret = iolink_master_write_isdu(&port->master, index, subindex, data, len);
        if(ret != IOLINK_MASTER_STATUS_PENDING)
        {
            return ret;
        }
    }

    return LW_IOLM_ERR_CYCLIC;
}

static int drive_fixture_read_isdu(lw_iolm_port_fixture_t* port,
                                   uint16_t index,
                                   uint8_t subindex,
                                   uint8_t* data,
                                   uint8_t* len,
                                   uint8_t* cycle)
{
    lw_iolm_conformance_result_t result;
    uint8_t i;
    int ret = iolink_master_read_isdu(&port->master, index, subindex, data, len);

    if(ret != IOLINK_MASTER_STATUS_PENDING)
    {
        return ret;
    }

    for(i = 0U; i < 80U; i++)
    {
        memset(&result, 0, sizeof(result));
        ret = tick_fixture(port, *cycle, &result);
        (*cycle)++;
        if(ret != LW_IOLM_STATUS_OK)
        {
            return ret;
        }
        ret = iolink_master_read_isdu(&port->master, index, subindex, data, len);
        if(ret != IOLINK_MASTER_STATUS_PENDING)
        {
            return ret;
        }
    }

    return LW_IOLM_ERR_CYCLIC;
}

int lw_iolm_conformance_run_profile(uint8_t m_seq_type,
                                    uint8_t pd_in_len,
                                    uint8_t pd_out_len,
                                    uint8_t pd_value,
                                    lw_iolm_conformance_result_t* result)
{
    lw_iolm_port_fixture_t port;
    uint8_t cycle;
    int ret;

    if(result == NULL)
    {
        return LW_IOLM_ERR_INVALID_ARG;
    }
    memset(result, 0, sizeof(*result));
    ret = init_fixture(&port, m_seq_type, pd_in_len, pd_out_len, pd_value);
    if(ret != LW_IOLM_STATUS_OK)
    {
        return ret;
    }

    for(cycle = 0U; cycle < 20U; cycle++)
    {
        ret = tick_fixture(&port, cycle, result);
        if(ret != LW_IOLM_STATUS_OK)
        {
            return ret;
        }
        if(result->master_state == IOLINK_MASTER_STATE_OPERATE)
        {
            return LW_IOLM_STATUS_OK;
        }
    }

    result->master_state = (int32_t)iolink_master_get_state(&port.master);
    return LW_IOLM_ERR_NO_OPERATE;
}

int lw_iolm_conformance_run_multi_profile(uint8_t port_count,
                                          uint8_t m_seq_type,
                                          uint8_t pd_in_len,
                                          uint8_t pd_out_len,
                                          uint8_t first_pd_value,
                                          lw_iolm_conformance_result_t* results)
{
    lw_iolm_port_fixture_t ports[LW_IOLM_MAX_PORTS];
    uint8_t completed[LW_IOLM_MAX_PORTS] = {0U};
    uint8_t complete_count = 0U;
    uint8_t port_idx;
    uint8_t cycle;
    int ret;

    if((results == NULL) || (port_count == 0U) || (port_count > LW_IOLM_MAX_PORTS))
    {
        return LW_IOLM_ERR_INVALID_ARG;
    }
    memset(results, 0, sizeof(lw_iolm_conformance_result_t) * port_count);

    for(port_idx = 0U; port_idx < port_count; port_idx++)
    {
        ret = init_fixture(&ports[port_idx],
                           m_seq_type,
                           pd_in_len,
                           pd_out_len,
                           (uint8_t)(first_pd_value + (uint8_t)(port_idx * 0x10U)));
        if(ret != LW_IOLM_STATUS_OK)
        {
            return ret;
        }
    }

    for(cycle = 0U; cycle < 20U; cycle++)
    {
        for(port_idx = 0U; port_idx < port_count; port_idx++)
        {
            if(completed[port_idx] != 0U)
            {
                continue;
            }
            ret = tick_fixture(&ports[port_idx], cycle, &results[port_idx]);
            if(ret != LW_IOLM_STATUS_OK)
            {
                return ret;
            }
            if(results[port_idx].master_state == IOLINK_MASTER_STATE_OPERATE)
            {
                completed[port_idx] = 1U;
                complete_count++;
            }
        }
        if(complete_count == port_count)
        {
            return LW_IOLM_STATUS_OK;
        }
    }

    for(port_idx = 0U; port_idx < port_count; port_idx++)
    {
        if(results[port_idx].master_state != IOLINK_MASTER_STATE_OPERATE)
        {
            results[port_idx].master_state = (int32_t)iolink_master_get_state(&ports[port_idx].master);
        }
    }
    return LW_IOLM_ERR_NO_OPERATE;
}

int lw_iolm_conformance_run_multi_direct_parameter_isolation(uint8_t* values, uint8_t value_count)
{
    lw_iolm_port_fixture_t ports[2];
    uint8_t cycles[2] = {0U};
    uint8_t write0 = 0xA1U;
    uint8_t write1 = 0xB2U;
    uint8_t read0 = 0U;
    uint8_t read1 = 0U;
    uint8_t len0 = 1U;
    uint8_t len1 = 1U;
    int ret;

    if((values == NULL) || (value_count < 2U))
    {
        return LW_IOLM_ERR_INVALID_ARG;
    }

    ret = init_fixture(&ports[0], IOLINK_MASTER_M_SEQ_TYPE_1_2, 2U, 1U, 0x51U);
    if(ret != LW_IOLM_STATUS_OK)
    {
        return ret;
    }
    ret = init_fixture(&ports[1], IOLINK_MASTER_M_SEQ_TYPE_1_2, 2U, 1U, 0x61U);
    if(ret != LW_IOLM_STATUS_OK)
    {
        return ret;
    }
    ret = drive_fixture_to_operate(&ports[0], &cycles[0]);
    if(ret != LW_IOLM_STATUS_OK)
    {
        return ret;
    }
    ret = drive_fixture_to_operate(&ports[1], &cycles[1]);
    if(ret != LW_IOLM_STATUS_OK)
    {
        return ret;
    }

    ret = drive_fixture_write_isdu(&ports[0],
                                   IOLINK_IDX_DIRECT_PARAMETERS_2,
                                   1U,
                                   &write0,
                                   1U,
                                   &cycles[0]);
    if(ret != IOLINK_MASTER_STATUS_OK)
    {
        return ret;
    }
    ret = drive_fixture_write_isdu(&ports[1],
                                   IOLINK_IDX_DIRECT_PARAMETERS_2,
                                   1U,
                                   &write1,
                                   1U,
                                   &cycles[1]);
    if(ret != IOLINK_MASTER_STATUS_OK)
    {
        return ret;
    }

    ret = drive_fixture_read_isdu(&ports[0],
                                  IOLINK_IDX_DIRECT_PARAMETERS_2,
                                  1U,
                                  &read0,
                                  &len0,
                                  &cycles[0]);
    if(ret != IOLINK_MASTER_STATUS_OK)
    {
        return ret;
    }
    ret = drive_fixture_read_isdu(&ports[1],
                                  IOLINK_IDX_DIRECT_PARAMETERS_2,
                                  1U,
                                  &read1,
                                  &len1,
                                  &cycles[1]);
    if(ret != IOLINK_MASTER_STATUS_OK)
    {
        return ret;
    }
    if((len0 != 1U) || (len1 != 1U))
    {
        return LW_IOLM_ERR_CYCLIC;
    }

    values[0] = read0;
    values[1] = read1;
    return ((read0 == write0) && (read1 == write1)) ? LW_IOLM_STATUS_OK : LW_IOLM_ERR_CYCLIC;
}
