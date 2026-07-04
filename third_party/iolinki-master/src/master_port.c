#include "master_internal.h"

#include "iolinki/crc.h"
#include "iolinki/frame.h"
#include "iolinki/protocol.h"

#include <string.h>

static const iolink_baudrate_t g_iolink_master_baudrate_scan[] = {
    IOLINK_BAUDRATE_COM3,
    IOLINK_BAUDRATE_COM2,
    IOLINK_BAUDRATE_COM1,
};

static bool iolink_master_startup_validation_required(const iolink_master_config_t* config)
{
    return config->validate_device_info ||
           (config->inspection_level != IOLINK_MASTER_INSPECTION_NO_CHECK);
}

static iolink_baudrate_t iolink_master_startup_baudrate(const iolink_master_port_t* port)
{
    const iolink_master_port_state_t* state = iolink_master_port_const_state(port);

    if(state->config.auto_baudrate)
    {
        return g_iolink_master_baudrate_scan[state->startup.baudrate_index];
    }

    return state->config.baudrate;
}

static uint8_t iolink_master_response_timeout_100us(const iolink_master_port_state_t* state)
{
    if(state->config.response_timeout_100us != 0U)
    {
        return state->config.response_timeout_100us;
    }

    return state->config.min_cycle_time;
}

static int iolink_master_set_mode(iolink_master_port_t* port, iolink_phy_mode_t mode)
{
    iolink_master_port_state_t* state = iolink_master_port_state(port);

    if(state->config.set_mode_checked != NULL)
    {
        return state->config.set_mode_checked(mode);
    }

    if((state->phy != NULL) && (state->phy->set_mode != NULL))
    {
        state->phy->set_mode(state->phy->user, mode);
    }

    return IOLINK_MASTER_STATUS_OK;
}

static int iolink_master_set_baudrate(iolink_master_port_t* port, iolink_baudrate_t baudrate)
{
    iolink_master_port_state_t* state = iolink_master_port_state(port);

    if(state->config.set_baudrate_checked != NULL)
    {
        return state->config.set_baudrate_checked(baudrate);
    }

    if((state->phy != NULL) && (state->phy->set_baudrate != NULL))
    {
        state->phy->set_baudrate(state->phy->user, baudrate);
    }

    return IOLINK_MASTER_STATUS_OK;
}

static int iolink_master_flush_rx(iolink_master_port_t* port)
{
    iolink_master_port_state_t* state = iolink_master_port_state(port);

    state->rx.len = 0U;
    if(state->config.flush_rx == NULL)
    {
        return IOLINK_MASTER_STATUS_OK;
    }

    return state->config.flush_rx();
}

static bool iolink_master_send_full(iolink_master_port_t* port, const uint8_t* data, size_t len)
{
    iolink_master_port_state_t* state = iolink_master_port_state(port);
    int sent;
    int ret;

    if(state->config.prepare_tx != NULL)
    {
        ret = state->config.prepare_tx();
        if(ret != IOLINK_MASTER_STATUS_OK)
        {
            state->diagnostics.send_errors++;
            state->state = IOLINK_MASTER_STATE_ERROR;
            return false;
        }
    }

    sent = state->phy->send(state->phy->user, data, len);

    if(state->config.prepare_rx != NULL)
    {
        ret = state->config.prepare_rx();
        if(ret != IOLINK_MASTER_STATUS_OK)
        {
            state->diagnostics.send_errors++;
            state->state = IOLINK_MASTER_STATE_ERROR;
            return false;
        }
    }

    if(sent == (int)len)
    {
        return true;
    }

    state->diagnostics.send_errors++;
    state->state = IOLINK_MASTER_STATE_ERROR;
    return false;
}

static bool iolink_master_wake_up(iolink_master_port_t* port)
{
    iolink_master_port_state_t* state = iolink_master_port_state(port);
    int ret;

    if(state->config.wake_up == NULL)
    {
        state->tx_buf[0] = IOLINK_MASTER_WAKEUP_BYTE;
        return iolink_master_send_full(port, state->tx_buf, 1U);
    }

    ret = state->config.wake_up();
    if(ret == IOLINK_MASTER_STATUS_OK)
    {
        return true;
    }

    state->diagnostics.send_errors++;
    state->state = IOLINK_MASTER_STATE_ERROR;
    return false;
}

static bool iolink_master_cycle_due_at(const iolink_master_port_t* port, uint32_t now_100us)
{
    const iolink_master_port_state_t* state = iolink_master_port_const_state(port);

    if((state->state != IOLINK_MASTER_STATE_OPERATE) || (state->config.min_cycle_time == 0U) ||
       !state->cycle_timer_valid)
    {
        return true;
    }

    return ((uint32_t)(now_100us - state->last_cycle_start_100us) >=
            (uint32_t)state->config.min_cycle_time);
}

static bool iolink_master_cycle_slipped_at(const iolink_master_port_t* port, uint32_t now_100us)
{
    const iolink_master_port_state_t* state = iolink_master_port_const_state(port);

    if((state->state != IOLINK_MASTER_STATE_OPERATE) || (state->config.min_cycle_time == 0U) ||
       !state->cycle_timer_valid)
    {
        return false;
    }

    return ((uint32_t)(now_100us - state->last_cycle_start_100us) >
            (uint32_t)state->config.min_cycle_time);
}

static uint32_t iolink_master_cycle_jitter_at(const iolink_master_port_t* port,
                                              uint32_t now_100us)
{
    const iolink_master_port_state_t* state = iolink_master_port_const_state(port);
    uint32_t elapsed;

    if((state->state != IOLINK_MASTER_STATE_OPERATE) || (state->config.min_cycle_time == 0U) ||
       !state->cycle_timer_valid)
    {
        return 0U;
    }

    elapsed = (uint32_t)(now_100us - state->last_cycle_start_100us);
    if(elapsed <= (uint32_t)state->config.min_cycle_time)
    {
        return 0U;
    }

    return (uint32_t)(elapsed - (uint32_t)state->config.min_cycle_time);
}

int iolink_master_get_next_tick_time(const iolink_master_port_t* port,
                                     uint32_t now_100us,
                                     uint32_t* out_next_100us)
{
    const iolink_master_port_state_t* state;
    uint32_t cycle_due;

    if((port == NULL) || (out_next_100us == NULL))
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    state = iolink_master_port_const_state(port);
    if((state->state == IOLINK_MASTER_STATE_INACTIVE) ||
       (state->state == IOLINK_MASTER_STATE_ERROR))
    {
        *out_next_100us = now_100us;
        return IOLINK_MASTER_STATUS_OK;
    }

    if(state->awaiting_response)
    {
        *out_next_100us = (now_100us >= state->response_deadline_100us)
                              ? now_100us
                              : state->response_deadline_100us;
        return IOLINK_MASTER_STATUS_OK;
    }

    if((state->state != IOLINK_MASTER_STATE_OPERATE) || (state->config.min_cycle_time == 0U) ||
       !state->cycle_timer_valid)
    {
        *out_next_100us = now_100us;
        return IOLINK_MASTER_STATUS_OK;
    }

    cycle_due = (uint32_t)(state->last_cycle_start_100us +
                           (uint32_t)state->config.min_cycle_time);
    *out_next_100us = (now_100us >= cycle_due) ? now_100us : cycle_due;
    return IOLINK_MASTER_STATUS_OK;
}

static int iolink_master_tick_common(iolink_master_port_t* port,
                                     iolink_master_tick_event_t event,
                                     bool pace_cycles,
                                     uint32_t now_100us)
{
    iolink_master_port_state_t* state;
    uint32_t cycle_count_before;
    int rx_ret;
    int timeout_ret;

    if((port == NULL) || (event > IOLINK_MASTER_TICK_RESPONSE_TIMEOUT))
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    rx_ret = iolink_master_poll_rx(port);
    if(rx_ret < 0)
    {
        return rx_ret;
    }

    state = iolink_master_port_state(port);
    if(event == IOLINK_MASTER_TICK_RESPONSE_TIMEOUT)
    {
        if(pace_cycles && !state->awaiting_response)
        {
            return rx_ret;
        }

        state->awaiting_response = false;
        timeout_ret = iolink_master_on_timeout(port);
        if(timeout_ret != IOLINK_MASTER_STATUS_OK)
        {
            return timeout_ret;
        }
    }

    if(event != IOLINK_MASTER_TICK_CYCLE_DUE)
    {
        return rx_ret;
    }

    if(pace_cycles && !iolink_master_cycle_due_at(port, now_100us))
    {
        return rx_ret;
    }

    cycle_count_before = state->cycle_count;
    iolink_master_process(port);

    /* iolink_master_process() increments cycle_count through the port pointer on a
       successful operate send; cppcheck does not model that side effect. */
    /* cppcheck-suppress knownConditionTrueFalse */
    if(pace_cycles && (state->cycle_count != cycle_count_before))
    {
        uint32_t jitter_100us = iolink_master_cycle_jitter_at(port, now_100us);

        if(iolink_master_cycle_slipped_at(port, now_100us))
        {
            state->diagnostics.cycle_slips++;
        }
        state->diagnostics.last_cycle_jitter_100us = jitter_100us;
        if(jitter_100us > state->diagnostics.max_cycle_jitter_100us)
        {
            state->diagnostics.max_cycle_jitter_100us = jitter_100us;
        }

        state->last_cycle_start_100us = now_100us;
        state->response_deadline_100us =
            (uint32_t)(now_100us + (uint32_t)iolink_master_response_timeout_100us(state));
        state->cycle_timer_valid = true;
        state->awaiting_response = true;
    }

    return rx_ret;
}

int iolink_master_init(iolink_master_port_t* port,
                       const iolink_phy_api_t* phy,
                       const iolink_master_config_t* config)
{
    int ret;

    if((port == NULL) || (phy == NULL) || (config == NULL) ||
       (config->pd_in_len > IOLINK_PD_IN_MAX_SIZE) ||
       (config->pd_out_len > IOLINK_PD_OUT_MAX_SIZE) ||
       (config->m_seq_type > IOLINK_MASTER_M_SEQ_TYPE_2_V) ||
       (config->baudrate > IOLINK_BAUDRATE_COM3) ||
       (config->port_mode > IOLINK_MASTER_PORT_MODE_DEACTIVATED) ||
       ((config->m_seq_type == IOLINK_MASTER_M_SEQ_TYPE_0) &&
        ((config->pd_in_len > 0U) || (config->pd_out_len > 0U))))
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    (void)memset(port, 0, sizeof(*port));
    iolink_master_port_state(port)->phy = phy;
    iolink_master_port_state(port)->config = *config;
    iolink_master_port_state(port)->od_len = iolink_master_od_len_for_type(config->m_seq_type);
    iolink_master_port_state(port)->pd_in_len = config->pd_in_len;
    iolink_master_port_state(port)->pd_out_len = config->pd_out_len;
    iolink_master_port_state(port)->startup.baudrate_index = 0U;
    iolink_master_port_state(port)->state = (config->port_mode == IOLINK_MASTER_PORT_MODE_IOLINK)
                      ? IOLINK_MASTER_STATE_STARTUP
                      : IOLINK_MASTER_STATE_INACTIVE;

    if(phy->init != NULL)
    {
        ret = phy->init(phy->user);
        if(ret != 0)
        {
            iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_ERROR;
            return ret;
        }
    }

    if(config->port_mode == IOLINK_MASTER_PORT_MODE_DEACTIVATED)
    {
        ret = iolink_master_set_mode(port, IOLINK_PHY_MODE_INACTIVE);
        if(ret != IOLINK_MASTER_STATUS_OK)
        {
            iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_ERROR;
            return ret;
        }
        return IOLINK_MASTER_STATUS_OK;
    }

    if((config->port_mode == IOLINK_MASTER_PORT_MODE_DI) ||
       (config->port_mode == IOLINK_MASTER_PORT_MODE_DQ))
    {
        ret = iolink_master_set_mode(port, IOLINK_PHY_MODE_SIO);
        if(ret != IOLINK_MASTER_STATUS_OK)
        {
            iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_ERROR;
            return ret;
        }
        return IOLINK_MASTER_STATUS_OK;
    }

    ret = iolink_master_flush_rx(port);
    if(ret != IOLINK_MASTER_STATUS_OK)
    {
        iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_ERROR;
        return ret;
    }

    ret = iolink_master_set_baudrate(port, iolink_master_startup_baudrate(port));
    if(ret != IOLINK_MASTER_STATUS_OK)
    {
        iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_ERROR;
        return ret;
    }

    ret = iolink_master_set_mode(port, IOLINK_PHY_MODE_SDCI);
    if(ret != IOLINK_MASTER_STATUS_OK)
    {
        iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_ERROR;
        return ret;
    }

    return IOLINK_MASTER_STATUS_OK;
}

int iolink_master_validate_phy_contract(const iolink_phy_api_t* phy,
                                        const iolink_master_config_t* config)
{
    if((phy == NULL) || (config == NULL) ||
       (config->port_mode > IOLINK_MASTER_PORT_MODE_DEACTIVATED))
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    switch(config->port_mode)
    {
    case IOLINK_MASTER_PORT_MODE_IOLINK:
        if((phy->send == NULL) || (phy->recv_byte == NULL) ||
           (config->set_mode_checked == NULL) ||
           (config->set_baudrate_checked == NULL) || (config->flush_rx == NULL) ||
           (config->prepare_tx == NULL) || (config->prepare_rx == NULL) ||
           (config->wake_up == NULL))
        {
            return IOLINK_MASTER_ERR_UNSUPPORTED_PHY;
        }
        return IOLINK_MASTER_STATUS_OK;
    case IOLINK_MASTER_PORT_MODE_DI:
        if((config->set_mode_checked == NULL) || (config->read_cq_line_checked == NULL))
        {
            return IOLINK_MASTER_ERR_UNSUPPORTED_PHY;
        }
        return IOLINK_MASTER_STATUS_OK;
    case IOLINK_MASTER_PORT_MODE_DQ:
        if((config->set_mode_checked == NULL) || (phy->set_cq_line == NULL))
        {
            return IOLINK_MASTER_ERR_UNSUPPORTED_PHY;
        }
        return IOLINK_MASTER_STATUS_OK;
    case IOLINK_MASTER_PORT_MODE_DEACTIVATED:
        if(config->set_mode_checked == NULL)
        {
            return IOLINK_MASTER_ERR_UNSUPPORTED_PHY;
        }
        return IOLINK_MASTER_STATUS_OK;
    default:
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }
}

int iolink_master_restart(iolink_master_port_t* port)
{
    const iolink_phy_api_t* phy;
    iolink_master_config_t config;

    if((port == NULL) || (iolink_master_port_state(port)->phy == NULL))
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    phy = iolink_master_port_state(port)->phy;
    config = iolink_master_port_state(port)->config;

    return iolink_master_init(port, phy, &config);
}

int iolink_master_on_timeout(iolink_master_port_t* port)
{
    int ret;

    if(port == NULL)
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    ret = iolink_master_flush_rx(port);
    if(ret != IOLINK_MASTER_STATUS_OK)
    {
        iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_ERROR;
        return ret;
    }

    if(iolink_master_port_state(port)->state != IOLINK_MASTER_STATE_STARTUP)
    {
        if(iolink_master_port_state(port)->state == IOLINK_MASTER_STATE_OPERATE)
        {
            iolink_master_port_state(port)->diagnostics.response_timeouts++;
            if(iolink_master_port_state(port)->diagnostics.rx_retry_count < IOLINK_MASTER_RX_RETRY_LIMIT)
            {
                iolink_master_port_state(port)->diagnostics.rx_retry_count++;
                return IOLINK_MASTER_STATUS_PENDING;
            }

            iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_ERROR;
            return IOLINK_MASTER_ERR_RETRY_LIMIT;
        }

        return IOLINK_MASTER_STATUS_OK;
    }

    if(iolink_master_port_state(port)->phy == NULL)
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    /*
     * Re-issue the wake-up request at the current baudrate before giving up on
     * it. A device can miss the first WURQ pulse; retrying the wake sequence is
     * spec-permitted and lets a slow-to-wake device still link up. Only advance
     * the baud scan (or error) once the per-baud wake budget is exhausted.
     */
    if(iolink_master_port_state(port)->startup.wake_attempts <
       iolink_master_port_state(port)->config.wake_retry_limit)
    {
        iolink_master_port_state(port)->startup.wake_attempts++;
        iolink_master_port_state(port)->startup.step = 0U;
        return IOLINK_MASTER_STATUS_PENDING;
    }

    if(iolink_master_port_state(port)->config.auto_baudrate &&
       (iolink_master_port_state(port)->startup.baudrate_index <
        (uint8_t)((sizeof(g_iolink_master_baudrate_scan) /
                   sizeof(g_iolink_master_baudrate_scan[0])) -
                  1U)))
    {
        iolink_master_port_state(port)->startup.baudrate_index++;
        iolink_master_port_state(port)->startup.step = 0U;
        iolink_master_port_state(port)->startup.wake_attempts = 0U;
        ret = iolink_master_set_baudrate(port, iolink_master_startup_baudrate(port));
        if(ret != IOLINK_MASTER_STATUS_OK)
        {
            iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_ERROR;
            return ret;
        }
        return IOLINK_MASTER_STATUS_PENDING;
    }

    iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_ERROR;
    return IOLINK_MASTER_ERR_RETRY_LIMIT;
}

int iolink_master_tick(iolink_master_port_t* port, bool response_timeout)
{
    return iolink_master_tick_event(port,
                                    response_timeout
                                        ? IOLINK_MASTER_TICK_RESPONSE_TIMEOUT
                                        : IOLINK_MASTER_TICK_CYCLE_DUE);
}

int iolink_master_tick_event(iolink_master_port_t* port, iolink_master_tick_event_t event)
{
    return iolink_master_tick_common(port, event, false, 0U);
}

int iolink_master_tick_at(iolink_master_port_t* port,
                          iolink_master_tick_event_t event,
                          uint32_t now_100us)
{
    return iolink_master_tick_common(port, event, true, now_100us);
}

void iolink_master_process(iolink_master_port_t* port)
{
    int frame_len;
    size_t od_pos;

    if((port == NULL) || (iolink_master_port_state(port)->phy == NULL) || (iolink_master_port_state(port)->phy->send == NULL))
    {
        return;
    }

    if(iolink_master_port_state(port)->state == IOLINK_MASTER_STATE_STARTUP)
    {
        if(iolink_master_port_state(port)->startup.step == IOLINK_MASTER_STARTUP_STEP_WAKE)
        {
            if(iolink_master_wake_up(port))
            {
                iolink_master_port_state(port)->startup.step++;
            }
            return;
        }

        if(iolink_master_port_state(port)->startup.step == IOLINK_MASTER_STARTUP_STEP_SEND_TYPE0)
        {
            /* Spec startup transition T1: first message is a Type-0 READ of the
               Direct Parameter page MinCycleTime octet (MC = 0xA2) on the page
               communication channel. */
            frame_len = iolink_frame_encode_type0(
                iolink_master_encode_master_command(true,
                                                    IOLINK_MASTER_MC_CHANNEL_PAGE,
                                                    IOLINK_MASTER_DPP1_OFF_MIN_CYCLE_TIME),
                iolink_master_port_state(port)->tx_buf,
                sizeof(iolink_master_port_state(port)->tx_buf));
            if(frame_len > 0)
            {
                if(iolink_master_send_full(port, iolink_master_port_state(port)->tx_buf, (size_t)frame_len))
                {
                    iolink_master_port_state(port)->startup.step++;
                }
            }
            return;
        }
    }

    if(iolink_master_port_state(port)->state == IOLINK_MASTER_STATE_PREOPERATE)
    {
        int ret;

        if((iolink_master_port_state(port)->isdu.op != IOLINK_MASTER_ISDU_OP_NONE) && !iolink_master_port_state(port)->isdu.done)
        {
            uint8_t od = 0U;
            iolink_master_isdu_fill_od(port, &od, 1U);
            frame_len = iolink_frame_encode_type0(od, iolink_master_port_state(port)->tx_buf, sizeof(iolink_master_port_state(port)->tx_buf));
            if(frame_len > 0)
            {
                (void)iolink_master_send_full(port, iolink_master_port_state(port)->tx_buf, (size_t)frame_len);
            }
            return;
        }

        if(iolink_master_startup_validation_required(&iolink_master_port_state(port)->config) && !iolink_master_port_state(port)->device_info.valid)
        {
            ret = iolink_master_read_device_info(port);
            if(ret == IOLINK_MASTER_STATUS_PENDING)
            {
                return;
            }
            if(ret < 0)
            {
                iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_ERROR;
                return;
            }
        }

        if(iolink_master_startup_validation_required(&iolink_master_port_state(port)->config) && (iolink_master_validate_device_info(port) != 0))
        {
            iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_ERROR;
            return;
        }

        /* Spec transition to OPERATE: write MasterCommand DeviceOperate (0x99,
           Table B.2) to Direct Parameter page address 0x00 on the page channel,
           i.e. a Type-0 WRITE frame (MC = 0x20, one OD data octet 0x99). */
        frame_len = iolink_frame_encode_type0_write(
            iolink_master_encode_master_command(false,
                                                IOLINK_MASTER_MC_CHANNEL_PAGE,
                                                IOLINK_MASTER_DPP1_OFF_MASTER_COMMAND),
            IOLINK_CMD_DEVICE_OPERATE,
            iolink_master_port_state(port)->tx_buf,
            sizeof(iolink_master_port_state(port)->tx_buf));
        if(frame_len > 0)
        {
            if(iolink_master_send_full(port, iolink_master_port_state(port)->tx_buf, (size_t)frame_len))
            {
                iolink_master_port_state(port)->startup.step++;
                iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_OPERATE;
            }
        }
        return;
    }

    if(iolink_master_port_state(port)->state == IOLINK_MASTER_STATE_OPERATE)
    {
        if((iolink_master_port_state(port)->config.m_seq_type == IOLINK_MASTER_M_SEQ_TYPE_0) &&
           (iolink_master_port_state(port)->config.pd_in_len == 0U) && (iolink_master_port_state(port)->pd_out_len == 0U))
        {
            uint8_t od = 0U;
            iolink_master_isdu_fill_od(port, &od, 1U);
            frame_len = iolink_frame_encode_type0(od, iolink_master_port_state(port)->tx_buf, sizeof(iolink_master_port_state(port)->tx_buf));
            if(frame_len > 0)
            {
                if(iolink_master_send_full(port, iolink_master_port_state(port)->tx_buf, (size_t)frame_len))
                {
                    iolink_master_port_state(port)->cycle_count++;
                }
            }
            return;
        }

        frame_len = iolink_frame_encode_type1_cycle(iolink_master_port_state(port)->pd_out,
                                                    iolink_master_port_state(port)->pd_out_len,
                                                    iolink_master_port_state(port)->od_len,
                                                    iolink_master_port_state(port)->tx_buf,
                                                    sizeof(iolink_master_port_state(port)->tx_buf));
        if(frame_len > 0)
        {
            od_pos = (size_t)IOLINK_M_SEQ_HEADER_LEN + iolink_master_port_state(port)->pd_out_len;
            iolink_master_isdu_fill_od(port, &iolink_master_port_state(port)->tx_buf[od_pos], iolink_master_port_state(port)->od_len);
            iolink_master_port_state(port)->tx_buf[frame_len - 1] = iolink_crc6(iolink_master_port_state(port)->tx_buf, (uint8_t)(frame_len - 1));

            if(iolink_master_send_full(port, iolink_master_port_state(port)->tx_buf, (size_t)frame_len))
            {
                iolink_master_port_state(port)->cycle_count++;
            }
        }
    }
}

int iolink_master_poll_rx(iolink_master_port_t* port)
{
    uint8_t byte;
    uint8_t expected_len;
    int recv_ret;
    int frame_ret;
    int frames = 0;

    if((port == NULL) || (iolink_master_port_state(port)->phy == NULL))
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    if(iolink_master_port_state(port)->phy->recv_byte == NULL)
    {
        return IOLINK_MASTER_STATUS_OK;
    }

    if((iolink_master_port_state(port)->state == IOLINK_MASTER_STATE_STARTUP) && (iolink_master_port_state(port)->startup.step >= IOLINK_MASTER_STARTUP_STEP_AWAIT_RESPONSE))
    {
        expected_len = IOLINK_M_SEQ_TYPE0_LEN;
    }
    else if(iolink_master_port_state(port)->state == IOLINK_MASTER_STATE_PREOPERATE)
    {
        expected_len = IOLINK_M_SEQ_TYPE0_LEN;
    }
    else if(iolink_master_port_state(port)->state == IOLINK_MASTER_STATE_OPERATE)
    {
        expected_len = (uint8_t)(1U + iolink_master_port_state(port)->config.pd_in_len + iolink_master_port_state(port)->od_len + 1U);
    }
    else
    {
        return IOLINK_MASTER_STATUS_OK;
    }

    for(;;)
    {
        recv_ret = iolink_master_port_state(port)->phy->recv_byte(
            iolink_master_port_state(port)->phy->user, &byte);
        if(recv_ret <= 0)
        {
            break;
        }

        if(iolink_master_port_state(port)->rx.len >= sizeof(iolink_master_port_state(port)->rx.buf))
        {
            iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_ERROR;
            iolink_master_port_state(port)->rx.len = 0U;
            return IOLINK_MASTER_ERR_CHECKSUM;
        }

        iolink_master_port_state(port)->rx.buf[iolink_master_port_state(port)->rx.len++] = byte;

        if(iolink_master_port_state(port)->rx.len >= expected_len)
        {
            frame_ret = iolink_master_on_rx(port, iolink_master_port_state(port)->rx.buf, iolink_master_port_state(port)->rx.len);
            iolink_master_port_state(port)->rx.len = 0U;

            if(frame_ret != 0)
            {
                return frame_ret;
            }

            frames++;
        }
    }

    if(recv_ret < 0)
    {
        iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_ERROR;
        iolink_master_port_state(port)->rx.len = 0U;
        return IOLINK_MASTER_ERR_FRAME;
    }

    return frames;
}

int iolink_master_on_rx(iolink_master_port_t* port, const uint8_t* data, uint8_t len)
{
    iolink_frame_operate_response_t resp;

    if((port == NULL) || (data == NULL) || (len == 0U))
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    if(iolink_master_port_state(port)->state == IOLINK_MASTER_STATE_STARTUP)
    {
        if(len != IOLINK_M_SEQ_TYPE0_LEN)
        {
            return IOLINK_MASTER_ERR_FRAME;
        }

        if(iolink_checksum_ck(data[0], 0U) != data[1])
        {
            iolink_master_port_state(port)->diagnostics.checksum_errors++;
            if(iolink_master_port_state(port)->diagnostics.rx_retry_count < IOLINK_MASTER_RX_RETRY_LIMIT)
            {
                iolink_master_port_state(port)->diagnostics.rx_retry_count++;
            }
            else
            {
                iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_ERROR;
            }
            return IOLINK_MASTER_ERR_CHECKSUM;
        }

        iolink_master_port_state(port)->diagnostics.rx_retry_count = 0U;
        iolink_master_port_state(port)->startup.wake_attempts = 0U;
        iolink_master_port_state(port)->awaiting_response = false;
        iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_PREOPERATE;
        return IOLINK_MASTER_STATUS_OK;
    }

    if(iolink_master_port_state(port)->state == IOLINK_MASTER_STATE_PREOPERATE)
    {
        if(len != IOLINK_M_SEQ_TYPE0_LEN)
        {
            return IOLINK_MASTER_ERR_FRAME;
        }

        if(iolink_checksum_ck(data[0], 0U) != data[1])
        {
            iolink_master_port_state(port)->diagnostics.checksum_errors++;
            if(iolink_master_port_state(port)->diagnostics.rx_retry_count < IOLINK_MASTER_RX_RETRY_LIMIT)
            {
                iolink_master_port_state(port)->diagnostics.rx_retry_count++;
            }
            else
            {
                iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_ERROR;
            }
            return IOLINK_MASTER_ERR_CHECKSUM;
        }

        iolink_master_port_state(port)->diagnostics.rx_retry_count = 0U;
        iolink_master_port_state(port)->awaiting_response = false;
        iolink_master_isdu_on_od(port, data, 1U);
        return IOLINK_MASTER_STATUS_OK;
    }

    if((iolink_master_port_state(port)->state == IOLINK_MASTER_STATE_OPERATE) &&
       (iolink_master_port_state(port)->config.m_seq_type == IOLINK_MASTER_M_SEQ_TYPE_0) &&
       (iolink_master_port_state(port)->config.pd_in_len == 0U) && (iolink_master_port_state(port)->pd_out_len == 0U))
    {
        if(len != IOLINK_M_SEQ_TYPE0_LEN)
        {
            return IOLINK_MASTER_ERR_FRAME;
        }

        if(iolink_checksum_ck(data[0], 0U) != data[1])
        {
            iolink_master_port_state(port)->diagnostics.checksum_errors++;
            if(iolink_master_port_state(port)->diagnostics.rx_retry_count < IOLINK_MASTER_RX_RETRY_LIMIT)
            {
                iolink_master_port_state(port)->diagnostics.rx_retry_count++;
            }
            else
            {
                iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_ERROR;
            }
            return IOLINK_MASTER_ERR_CHECKSUM;
        }

        iolink_master_port_state(port)->diagnostics.rx_retry_count = 0U;
        iolink_master_port_state(port)->awaiting_response = false;
        iolink_master_isdu_on_od(port, data, 1U);
        return IOLINK_MASTER_STATUS_OK;
    }

    if(iolink_frame_decode_operate_response(data,
                                            len,
                                            iolink_master_port_state(port)->config.pd_in_len,
                                            iolink_master_port_state(port)->od_len,
                                            &resp) != 0)
    {
        return IOLINK_MASTER_ERR_FRAME;
    }

    if(!resp.checksum_ok)
    {
        iolink_master_port_state(port)->diagnostics.checksum_errors++;
        if(iolink_master_port_state(port)->diagnostics.rx_retry_count < IOLINK_MASTER_RX_RETRY_LIMIT)
        {
            iolink_master_port_state(port)->diagnostics.rx_retry_count++;
        }
        else
        {
            iolink_master_port_state(port)->state = IOLINK_MASTER_STATE_ERROR;
        }
        return IOLINK_MASTER_ERR_CHECKSUM;
    }

    iolink_master_port_state(port)->diagnostics.rx_retry_count = 0U;
    iolink_master_port_state(port)->awaiting_response = false;
    iolink_master_port_state(port)->diagnostics.od_status = resp.status;

    /*
     * Dispatch on the rising edge of the OD Event flag. This turns event
     * handling from "poll diagnostics.event_pending yourself" into a
     * notification: the application reacts by reading event details (which then
     * dispatches each decoded event through config.event_handler).
     */
    if(resp.event_pending && !iolink_master_port_state(port)->diagnostics.event_pending &&
       (iolink_master_port_state(port)->config.event_pending_handler != NULL))
    {
        iolink_master_port_state(port)->config.event_pending_handler(
            iolink_master_port_state(port)->config.event_user);
    }
    iolink_master_port_state(port)->diagnostics.event_pending = resp.event_pending;

    if(resp.pd_valid)
    {
        (void)memcpy(iolink_master_port_state(port)->pd_in, resp.pd, resp.pd_len);
        iolink_master_port_state(port)->pd_in_len = resp.pd_len;
        iolink_master_port_state(port)->pd_valid = true;
    }

    iolink_master_isdu_on_od(port, resp.od, resp.od_len);

    return IOLINK_MASTER_STATUS_OK;
}

iolink_master_state_t iolink_master_get_state(const iolink_master_port_t* port)
{
    const iolink_master_port_state_t* state;

    if(port == NULL)
    {
        return IOLINK_MASTER_STATE_ERROR;
    }

    state = iolink_master_port_const_state(port);
    return state->state;
}

int iolink_master_get_pd_in(const iolink_master_port_t* port,
                            uint8_t* buffer,
                            uint8_t buffer_len,
                            uint8_t* out_len)
{
    const iolink_master_port_state_t* state;

    if((port == NULL) || (buffer == NULL) || (out_len == NULL))
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    state = iolink_master_port_const_state(port);

    if(buffer_len < state->pd_in_len)
    {
        *out_len = state->pd_in_len;
        return IOLINK_MASTER_ERR_BUFFER_TOO_SMALL;
    }

    *out_len = state->pd_in_len;

    if(!state->pd_valid)
    {
        return IOLINK_MASTER_STATUS_PENDING;
    }

    (void)memcpy(buffer, state->pd_in, state->pd_in_len);
    return IOLINK_MASTER_STATUS_OK;
}

int iolink_master_get_od_status(const iolink_master_port_t* port, uint8_t* status)
{
    const iolink_master_port_state_t* state;

    if((port == NULL) || (status == NULL))
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    state = iolink_master_port_const_state(port);
    *status = state->diagnostics.od_status;
    return IOLINK_MASTER_STATUS_OK;
}

uint8_t iolink_master_get_device_status(const iolink_master_port_t* port)
{
    const iolink_master_port_state_t* state;

    if(port == NULL)
    {
        return IOLINK_DEVICE_STATUS_FAILURE;
    }

    state = iolink_master_port_const_state(port);
    return (uint8_t)(state->diagnostics.od_status & IOLINK_OD_STATUS_DEVICE_MASK);
}

int iolink_master_get_diagnostics(const iolink_master_port_t* port,
                                  iolink_master_diagnostics_t* diagnostics)
{
    const iolink_master_port_state_t* state;
    uint32_t error_count;
    uint32_t total_count;

    if((port == NULL) || (diagnostics == NULL))
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    state = iolink_master_port_const_state(port);
    *diagnostics = state->diagnostics;
    diagnostics->supply_voltage_mv =
        ((state->phy != NULL) && (state->phy->get_voltage_mv != NULL))
            ? state->phy->get_voltage_mv(state->phy->user)
            : 0;
    diagnostics->short_circuit =
        ((state->phy != NULL) && (state->phy->is_short_circuit != NULL))
            ? state->phy->is_short_circuit(state->phy->user)
            : false;
    error_count = diagnostics->checksum_errors + diagnostics->send_errors +
                  diagnostics->response_timeouts;
    total_count = state->cycle_count + error_count;
    diagnostics->link_quality_percent =
        (total_count == 0U)
            ? 100U
            : (uint8_t)((state->cycle_count * 100U) / total_count);
    return IOLINK_MASTER_STATUS_OK;
}

int iolink_master_get_timing(const iolink_master_port_t* port, iolink_master_timing_t* timing)
{
    const iolink_master_port_state_t* state;

    if((port == NULL) || (timing == NULL))
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    state = iolink_master_port_const_state(port);
    timing->cycle_timer_valid = state->cycle_timer_valid;
    timing->awaiting_response = state->awaiting_response;
    timing->min_cycle_time_100us = state->config.min_cycle_time;
    timing->last_cycle_start_100us = state->last_cycle_start_100us;
    timing->response_deadline_100us = state->response_deadline_100us;

    return IOLINK_MASTER_STATUS_OK;
}

int iolink_master_set_pd_out(iolink_master_port_t* port, const uint8_t* data, uint8_t len)
{
    if((port == NULL) || ((data == NULL) && (len > 0U)))
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    if((len > IOLINK_PD_OUT_MAX_SIZE) || (len != iolink_master_port_state(port)->config.pd_out_len))
    {
        return IOLINK_MASTER_ERR_BUFFER_TOO_SMALL;
    }

    if(len > 0U)
    {
        (void)memcpy(iolink_master_port_state(port)->pd_out, data, len);
    }
    iolink_master_port_state(port)->pd_out_len = len;
    return IOLINK_MASTER_STATUS_OK;
}
