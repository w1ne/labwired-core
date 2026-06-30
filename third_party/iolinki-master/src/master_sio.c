#include "master_internal.h"

int iolink_master_set_dq(iolink_master_port_t* port, bool level)
{
    iolink_master_port_state_t* state;

    if(port == NULL)
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    state = iolink_master_port_state(port);
    if(state->config.port_mode != IOLINK_MASTER_PORT_MODE_DQ)
    {
        return IOLINK_MASTER_SIO_ERR_WRONG_MODE;
    }

    if((state->phy == NULL) || (state->phy->set_cq_line == NULL))
    {
        return IOLINK_MASTER_SIO_ERR_UNSUPPORTED_PHY;
    }

    state->phy->set_cq_line(state->phy->user, level ? 1U : 0U);
    return IOLINK_MASTER_STATUS_OK;
}

int iolink_master_get_di(const iolink_master_port_t* port, bool* level)
{
    const iolink_master_port_state_t* state;
    int cq_level;

    if((port == NULL) || (level == NULL))
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    state = iolink_master_port_const_state(port);
    if(state->config.port_mode != IOLINK_MASTER_PORT_MODE_DI)
    {
        return IOLINK_MASTER_SIO_ERR_WRONG_MODE;
    }

    if((state->config.read_cq_line_checked == NULL) && (state->config.read_cq_line == NULL))
    {
        return IOLINK_MASTER_SIO_ERR_UNSUPPORTED_PHY;
    }

    cq_level = (state->config.read_cq_line_checked != NULL)
                   ? state->config.read_cq_line_checked()
                   : state->config.read_cq_line();
    if(cq_level < 0)
    {
        return IOLINK_MASTER_SIO_ERR_UNSUPPORTED_PHY;
    }

    *level = (cq_level != 0);
    return IOLINK_MASTER_STATUS_OK;
}

int iolink_master_set_port_mode(iolink_master_port_t* port, iolink_master_port_mode_t mode)
{
    const iolink_phy_api_t* phy;
    iolink_master_config_t config;

    if((port == NULL) || (mode > IOLINK_MASTER_PORT_MODE_DEACTIVATED) ||
       (iolink_master_port_state(port)->phy == NULL))
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    phy = iolink_master_port_state(port)->phy;
    config = iolink_master_port_state(port)->config;
    config.port_mode = mode;

    return iolink_master_init(port, phy, &config);
}
