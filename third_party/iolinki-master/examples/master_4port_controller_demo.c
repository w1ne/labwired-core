#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "iolinki_master/master.h"

static int g_iolink_sends;
static int g_dq_level;
static int g_di_level = 1;
static iolink_phy_mode_t g_modes[4];

static int phy0_send(void* user, const uint8_t* data, size_t len)
{
    (void)user;
    if((data == NULL) || (len == 0U))
    {
        return -1;
    }

    g_iolink_sends++;
    return (int)len;
}

static void phy0_set_mode(void* user, iolink_phy_mode_t mode)
{
    (void)user;
    g_modes[0] = mode;
}

static void phy1_set_mode(void* user, iolink_phy_mode_t mode)
{
    (void)user;
    g_modes[1] = mode;
}

static void phy2_set_mode(void* user, iolink_phy_mode_t mode)
{
    (void)user;
    g_modes[2] = mode;
}

static void phy3_set_mode(void* user, iolink_phy_mode_t mode)
{
    (void)user;
    g_modes[3] = mode;
}

static int phy1_read_cq(void)
{
    return g_di_level;
}

static void phy2_set_cq(void* user, uint8_t level)
{
    (void)user;
    g_dq_level = (level != 0U) ? 1 : 0;
}

int main(void)
{
    iolink_master_controller_t controller;
    iolink_master_port_t ports[4];
    bool di_level = false;
    const iolink_phy_api_t phys[4] = {
        {.set_mode = phy0_set_mode, .send = phy0_send},
        {.set_mode = phy1_set_mode},
        {.set_mode = phy2_set_mode, .set_cq_line = phy2_set_cq},
        {.set_mode = phy3_set_mode},
    };
    const iolink_master_config_t configs[4] = {
        {
            .port_mode = IOLINK_MASTER_PORT_MODE_IOLINK,
            .m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_0,
            .baudrate = IOLINK_BAUDRATE_COM3,
            .min_cycle_time = 20U,
        },
        {
            .port_mode = IOLINK_MASTER_PORT_MODE_DI,
            .m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_0,
            .baudrate = IOLINK_BAUDRATE_COM3,
            .read_cq_line = phy1_read_cq,
        },
        {
            .port_mode = IOLINK_MASTER_PORT_MODE_DQ,
            .m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_0,
            .baudrate = IOLINK_BAUDRATE_COM3,
        },
        {
            .port_mode = IOLINK_MASTER_PORT_MODE_DEACTIVATED,
            .m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_0,
            .baudrate = IOLINK_BAUDRATE_COM3,
        },
    };

    memset(ports, 0, sizeof(ports));
    memset(g_modes, 0, sizeof(g_modes));

    if(iolink_master_controller_init(&controller, ports, 4U, phys, configs) !=
       IOLINK_MASTER_STATUS_OK)
    {
        return 1;
    }

    if((g_modes[0] != IOLINK_PHY_MODE_SDCI) || (g_modes[1] != IOLINK_PHY_MODE_SIO) ||
       (g_modes[2] != IOLINK_PHY_MODE_SIO) || (g_modes[3] != IOLINK_PHY_MODE_INACTIVE))
    {
        return 2;
    }

    if(iolink_master_get_di(&ports[1], &di_level) != IOLINK_MASTER_STATUS_OK)
    {
        return 3;
    }

    if(!di_level)
    {
        return 4;
    }

    if(iolink_master_set_dq(&ports[2], true) != IOLINK_MASTER_STATUS_OK)
    {
        return 5;
    }

    if(g_dq_level != 1)
    {
        return 6;
    }

    if(iolink_master_controller_tick_at(&controller, 100U) != IOLINK_MASTER_STATUS_OK)
    {
        return 7;
    }

    if(g_iolink_sends != 1)
    {
        return 8;
    }

    printf("iolinki-master 4-port demo: io-link-sends=%d di=%u dq=%d\n",
           g_iolink_sends,
           di_level ? 1U : 0U,
           g_dq_level);

    return 0;
}
