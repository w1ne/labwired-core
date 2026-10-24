/*
 * Copyright (C) 2026 Andrii Shylenko
 * SPDX-License-Identifier: GPL-3.0-or-later
 *
 * This file is part of iolinki-master.
 * See LICENSE for details.
 *
 * Minimal IO-Link master sample for native_sim.
 *
 * The master:
 *   - drives a trivial in-process fake PHY (no hardware/devicetree needed),
 *   - initializes a port with a small fixed-PD configuration,
 *   - advances the link through startup into OPERATE,
 *   - reads back one process-data cycle and the OD status.
 *
 * The fake PHY answers the master's wake-up, Type-0 identification and cyclic
 * requests just enough to reach OPERATE, mirroring examples/master_loopback_demo.c
 * from the host build. Being pure C with no Zephyr APIs, it runs unchanged on
 * native_sim.
 */

#include <zephyr/kernel.h>
#include <zephyr/logging/log.h>
#include <string.h>

#include "iolinki/crc.h"
#include "iolinki/frame.h"
#include "iolinki/protocol.h"
#include "iolinki_master/master.h"

LOG_MODULE_REGISTER(iolink_master_sample, LOG_LEVEL_INF);

static uint8_t g_rx_queue[64];
static uint8_t g_rx_len;
static uint8_t g_rx_pos;
static iolink_baudrate_t g_baudrate;
static iolink_phy_mode_t g_mode;

static void queue_bytes(const uint8_t* data, uint8_t len)
{
    memcpy(g_rx_queue, data, len);
    g_rx_len = len;
    g_rx_pos = 0U;
}

static int demo_phy_init(void* user)
{
    (void)user;
    g_rx_len = 0U;
    g_rx_pos = 0U;
    g_baudrate = IOLINK_BAUDRATE_COM3;
    g_mode = IOLINK_PHY_MODE_INACTIVE;
    return 0;
}

static void demo_phy_set_mode(void* user, iolink_phy_mode_t mode)
{
    (void)user;
    g_mode = mode;
}

static void demo_phy_set_baudrate(void* user, iolink_baudrate_t baudrate)
{
    (void)user;
    g_baudrate = baudrate;
}

static int demo_phy_recv_byte(void* user, uint8_t* byte)
{
    (void)user;
    if(byte == NULL)
    {
        return -1;
    }

    if(g_rx_pos >= g_rx_len)
    {
        return 0;
    }

    *byte = g_rx_queue[g_rx_pos++];
    return 1;
}

static int demo_phy_send(void* user, const uint8_t* data, size_t len)
{
    uint8_t response[8] = {0U};

    (void)user;
    if((data == NULL) || (len == 0U))
    {
        return -1;
    }

    if((len == 1U) && (data[0] == 0x55U))
    {
        return (int)len;
    }

    /* Startup probe (7.3.2.2 T1): Type-0 READ of a Direct Parameter octet on the
       page channel (MC 0xA2). Answer with a valid 2-octet Type-0 frame. */
    if((len == IOLINK_M_SEQ_TYPE0_LEN) && ((data[0] & IOLINK_MC_RW_MASK) != 0U) &&
       ((data[0] & IOLINK_MC_COMM_CHANNEL_MASK) == 0x20U))
    {
        response[0] = 0x00U;
        response[1] = iolink_checksum6(response, 2U);
        queue_bytes(response, 2U);
        return (int)len;
    }

    /* Transition to OPERATE: Type-0 DeviceOperate write (MC 0x20, OD 0x99).
       Figure A.5: the device answers with the CKS octet alone. */
    if((len == IOLINK_M_SEQ_MIN_LEN) && (data[0] == 0x20U) &&
       (data[IOLINK_M_SEQ_HEADER_LEN] == IOLINK_CMD_DEVICE_OPERATE))
    {
        response[0] = 0x2DU;
        queue_bytes(response, 1U);
        return (int)len;
    }

    /* TYPE_2_1 cyclic request: MC, CKT, PD-out(1), OD(1) -> 4 octets (Table A.10). */
    if(len == 4U)
    {
        if((data[0] & IOLINK_MC_RW_MASK) != 0U)
        {
            /* Figures A.9/A.10 READ reply: [OD][PD-in] CKS, no status octet;
               flags live in CKS bits 7/6 (A.1.5). */
            response[0] = 0x00U;
            response[1] = 0x5AU;
            response[2] = iolink_checksum6(response, 3U);
            queue_bytes(response, 3U);
        }
        else
        {
            /* Figures A.9/A.10 WRITE reply: [PD-in] CKS, no OD and no status
               octet; flags live in CKS bits 7/6 (A.1.5). */
            response[0] = 0x5AU;
            response[1] = iolink_checksum6(response, 2U);
            queue_bytes(response, 2U);
        }
        return (int)len;
    }

    return -1;
}

static const iolink_phy_api_t g_demo_phy = {
    .init = demo_phy_init,
    .set_mode = demo_phy_set_mode,
    .set_baudrate = demo_phy_set_baudrate,
    .send = demo_phy_send,
    .recv_byte = demo_phy_recv_byte,
};

int main(void)
{
    LOG_INF("Starting iolinki IO-Link master sample");

    iolink_master_port_t port;
    uint8_t pd_out[1] = {0x11U};
    uint8_t pd_in[1] = {0U};
    uint8_t pd_in_len = sizeof(pd_in);
    uint8_t od_status = 0U;
    iolink_master_config_t config = {
        .port_mode = IOLINK_MASTER_PORT_MODE_IOLINK,
        .m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_2_1,
        .baudrate = IOLINK_BAUDRATE_COM3,
        .min_cycle_time = 20U,
        .pd_in_len = sizeof(pd_in),
        .pd_out_len = sizeof(pd_out),
        .auto_baudrate = false,
    };

    if(iolink_master_init(&port, &g_demo_phy, &config) != 0)
    {
        LOG_ERR("iolink_master_init failed");
        return -1;
    }

    if((g_mode != IOLINK_PHY_MODE_SDCI) || (g_baudrate != IOLINK_BAUDRATE_COM3))
    {
        LOG_ERR("PHY not brought up into SDCI/COM3");
        return -1;
    }

    if(iolink_master_set_pd_out(&port, pd_out, sizeof(pd_out)) != 0)
    {
        LOG_ERR("iolink_master_set_pd_out failed");
        return -1;
    }

    /* Advance through startup identification into PREOPERATE. */
    iolink_master_process(&port);
    iolink_master_process(&port);
    (void)iolink_master_poll_rx(&port);

    if(iolink_master_get_state(&port) != IOLINK_MASTER_STATE_PREOPERATE)
    {
        LOG_ERR("port did not reach PREOPERATE");
        return -1;
    }

    /* Transition into OPERATE and run one cyclic exchange. */
    iolink_master_process(&port);
    /* Figure A.5: consume the CKS-only DeviceOperate reply, then enter OPERATE. */
    (void)iolink_master_poll_rx(&port);
    if(iolink_master_get_state(&port) != IOLINK_MASTER_STATE_OPERATE)
    {
        LOG_ERR("port did not reach OPERATE");
        return -1;
    }

    iolink_master_process(&port);
    (void)iolink_master_poll_rx(&port);

    if(iolink_master_get_pd_in(&port, pd_in, sizeof(pd_in), &pd_in_len) != 0)
    {
        LOG_ERR("iolink_master_get_pd_in failed");
        return -1;
    }

    (void)iolink_master_get_od_status(&port, &od_status);

    LOG_INF("Master reached state=%u pd_in=0x%02X od_status=0x%02X",
            (unsigned)iolink_master_get_state(&port),
            (unsigned)pd_in[0],
            (unsigned)od_status);

    return 0;
}
