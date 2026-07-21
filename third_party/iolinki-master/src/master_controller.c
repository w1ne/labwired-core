#include "master_internal.h"

#include <string.h>

int iolink_master_controller_init(iolink_master_controller_t* controller,
                                  iolink_master_port_t* ports,
                                  uint8_t port_count,
                                  const iolink_phy_api_t* phys,
                                  const iolink_master_config_t* configs)
{
    iolink_master_controller_state_t* state;
    uint8_t i;
    int ret;

    if((controller == NULL) || (ports == NULL) || (port_count == 0U) || (phys == NULL) ||
       (configs == NULL))
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    (void)memset(controller, 0, sizeof(*controller));
    state = iolink_master_controller_state(controller);
    state->ports = ports;
    state->port_count = port_count;

    for(i = 0U; i < port_count; i++)
    {
        ret = iolink_master_init(&ports[i], &phys[i], &configs[i]);
        if(ret != 0)
        {
            state->port_count = i;
            return ret;
        }
    }

    return IOLINK_MASTER_STATUS_OK;
}

int iolink_master_controller_tick(iolink_master_controller_t* controller,
                                  const bool* response_timeouts)
{
    iolink_master_controller_state_t* state;
    uint8_t i;
    int ret;
    int first_error = IOLINK_MASTER_STATUS_OK;
    iolink_master_tick_event_t event;

    if(controller == NULL)
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    state = iolink_master_controller_state(controller);
    for(i = 0U; i < state->port_count; i++)
    {
        event = ((response_timeouts != NULL) && response_timeouts[i])
                    ? IOLINK_MASTER_TICK_RESPONSE_TIMEOUT
                    : IOLINK_MASTER_TICK_CYCLE_DUE;
        ret = iolink_master_tick_event(&state->ports[i], event);
        if((ret < 0) && (first_error == 0))
        {
            first_error = ret;
        }
    }

    return first_error;
}

int iolink_master_controller_get_port_count(const iolink_master_controller_t* controller,
                                            uint8_t* out_count)
{
    if((controller == NULL) || (out_count == NULL))
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    *out_count = iolink_master_controller_const_state(controller)->port_count;
    return IOLINK_MASTER_STATUS_OK;
}

int iolink_master_controller_get_port(iolink_master_controller_t* controller,
                                      uint8_t index,
                                      iolink_master_port_t** out_port)
{
    iolink_master_controller_state_t* state;

    if((controller == NULL) || (out_port == NULL))
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    state = iolink_master_controller_state(controller);
    if(index >= state->port_count)
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    *out_port = &state->ports[index];
    return IOLINK_MASTER_STATUS_OK;
}

int iolink_master_controller_tick_events(iolink_master_controller_t* controller,
                                         const iolink_master_tick_event_t* events)
{
    iolink_master_controller_state_t* state;
    uint8_t i;
    int ret;
    int first_error = IOLINK_MASTER_STATUS_OK;
    iolink_master_tick_event_t event;

    if(controller == NULL)
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    state = iolink_master_controller_state(controller);
    for(i = 0U; i < state->port_count; i++)
    {
        event = (events != NULL) ? events[i] : IOLINK_MASTER_TICK_CYCLE_DUE;
        ret = iolink_master_tick_event(&state->ports[i], event);
        if((ret < 0) && (first_error == 0))
        {
            first_error = ret;
        }
    }

    return first_error;
}

int iolink_master_controller_tick_at(iolink_master_controller_t* controller, uint32_t now_100us)
{
    iolink_master_controller_state_t* state;
    uint8_t i;
    int ret;
    int first_error = IOLINK_MASTER_STATUS_OK;

    if(controller == NULL)
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    state = iolink_master_controller_state(controller);
    for(i = 0U; i < state->port_count; i++)
    {
        ret = iolink_master_tick_at(&state->ports[i],
                                    iolink_master_response_due_at(&state->ports[i], now_100us)
                                        ? IOLINK_MASTER_TICK_RESPONSE_TIMEOUT
                                        : IOLINK_MASTER_TICK_CYCLE_DUE,
                                    now_100us);
        if((ret < 0) && (first_error == 0))
        {
            first_error = ret;
        }
    }

    return first_error;
}

int iolink_master_controller_get_next_tick_time(const iolink_master_controller_t* controller,
                                                uint32_t now_100us,
                                                uint32_t* out_next_100us)
{
    const iolink_master_controller_state_t* state;
    uint8_t i;
    uint32_t port_next;

    if((controller == NULL) || (out_next_100us == NULL))
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    state = iolink_master_controller_const_state(controller);
    if(state->port_count == 0U)
    {
        *out_next_100us = now_100us;
        return IOLINK_MASTER_STATUS_OK;
    }

    *out_next_100us = UINT32_MAX;
    for(i = 0U; i < state->port_count; i++)
    {
        if(iolink_master_get_next_tick_time(&state->ports[i], now_100us, &port_next) !=
           IOLINK_MASTER_STATUS_OK)
        {
            return IOLINK_MASTER_ERR_INVALID_ARG;
        }
        if(port_next < *out_next_100us)
        {
            *out_next_100us = port_next;
        }
    }

    return IOLINK_MASTER_STATUS_OK;
}
