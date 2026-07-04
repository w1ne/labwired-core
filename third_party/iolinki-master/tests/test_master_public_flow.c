#include <setjmp.h>
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#include <cmocka.h>

#include "iolinki/crc.h"
#include "iolinki/protocol.h"
#include "iolinki_master/master.h"

static uint8_t g_rx_queue[16];
static uint8_t g_rx_len;
static uint8_t g_rx_pos;
static int g_send_calls;
static uint8_t g_sent[8][64];
static size_t g_sent_len[8];

static int fake_send(void* user, const uint8_t* data, size_t len)
{
    (void)user;
    assert_non_null(data);
    assert_in_range(g_send_calls, 0, 7);
    assert_in_range(len, 1U, sizeof(g_sent[0]));

    memcpy(g_sent[g_send_calls], data, len);
    g_sent_len[g_send_calls] = len;
    g_send_calls++;
    return (int)len;
}

static int fake_recv_byte(void* user, uint8_t* byte)
{
    (void)user;
    assert_non_null(byte);

    if(g_rx_pos >= g_rx_len)
    {
        return 0;
    }

    *byte = g_rx_queue[g_rx_pos++];
    return 1;
}

static void queue_bytes(const uint8_t* data, uint8_t len)
{
    memcpy(g_rx_queue, data, len);
    g_rx_len = len;
    g_rx_pos = 0U;
}

static const iolink_phy_api_t g_phy = {
    .send = fake_send,
    .recv_byte = fake_recv_byte,
};

static int reset_fixture(void** state)
{
    (void)state;
    memset(g_rx_queue, 0, sizeof(g_rx_queue));
    memset(g_sent, 0, sizeof(g_sent));
    memset(g_sent_len, 0, sizeof(g_sent_len));
    g_rx_len = 0U;
    g_rx_pos = 0U;
    g_send_calls = 0;
    return 0;
}

static void test_public_api_drives_startup_and_latches_process_data(void** state)
{
    const iolink_master_config_t config = {
        .port_mode = IOLINK_MASTER_PORT_MODE_IOLINK,
        .m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_1_1,
        .baudrate = IOLINK_BAUDRATE_COM3,
        .min_cycle_time = 20U,
        .pd_in_len = 1U,
        .pd_out_len = 0U,
    };
    iolink_master_port_t port;
    uint8_t startup_resp[2] = {0U};
    const uint8_t operate_resp[] = {0x20U, 0xA5U, 0x00U, 0x0DU};
    uint8_t pd[1] = {0U};
    uint8_t len = 0U;

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_phy, &config), 0);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_STARTUP);

    assert_int_equal(iolink_master_tick(&port, false), 0);
    assert_int_equal(g_send_calls, 1);
    assert_int_equal(g_sent_len[0], 1U);
    assert_int_equal(g_sent[0][0], 0x55U);

    assert_int_equal(iolink_master_tick(&port, false), 0);
    assert_int_equal(g_send_calls, 2);

    startup_resp[1] = iolink_checksum_ck(startup_resp[0], 0U);
    queue_bytes(startup_resp, sizeof(startup_resp));
    assert_int_equal(iolink_master_tick(&port, false), 1);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);
    /* Transition to OPERATE is the Type-0 DeviceOperate write (MC 0x20, OD 0x99). */
    assert_int_equal(g_sent[g_send_calls - 1][0],
                     iolink_master_encode_master_command(false, IOLINK_MASTER_MC_CHANNEL_PAGE, 0x00U));
    assert_int_equal(g_sent[g_send_calls - 1][1], IOLINK_CMD_DEVICE_OPERATE);

    queue_bytes(operate_resp, sizeof(operate_resp));
    assert_int_equal(iolink_master_tick(&port, false), 1);
    assert_int_equal(iolink_master_get_pd_in(&port, pd, sizeof(pd), &len), 0);
    assert_int_equal(len, 1U);
    assert_int_equal(pd[0], 0xA5U);
}

static void test_public_api_exposes_scheduler_timing_state(void** state)
{
    const iolink_master_config_t config = {
        .port_mode = IOLINK_MASTER_PORT_MODE_IOLINK,
        .m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_0,
        .baudrate = IOLINK_BAUDRATE_COM3,
        .min_cycle_time = 20U,
    };
    iolink_master_port_t port;
    iolink_master_timing_t timing;
    uint8_t startup_resp[2] = {0U};

    (void)state;

    assert_int_equal(iolink_master_get_timing(NULL, &timing), IOLINK_MASTER_ERR_INVALID_ARG);
    assert_int_equal(iolink_master_init(&port, &g_phy, &config), IOLINK_MASTER_STATUS_OK);
    assert_int_equal(iolink_master_get_timing(&port, &timing), IOLINK_MASTER_STATUS_OK);
    assert_false(timing.cycle_timer_valid);
    assert_false(timing.awaiting_response);
    assert_int_equal(timing.min_cycle_time_100us, 20U);

    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    startup_resp[1] = iolink_checksum_ck(startup_resp[0], 0U);
    assert_int_equal(iolink_master_on_rx(&port, startup_resp, sizeof(startup_resp)),
                     IOLINK_MASTER_STATUS_OK);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);

    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, 100U),
                     IOLINK_MASTER_STATUS_OK);
    assert_int_equal(iolink_master_get_timing(&port, &timing), IOLINK_MASTER_STATUS_OK);
    assert_true(timing.cycle_timer_valid);
    assert_true(timing.awaiting_response);
    assert_int_equal(timing.last_cycle_start_100us, 100U);
    assert_int_equal(timing.response_deadline_100us, 120U);

    startup_resp[0] = 0x00U;
    startup_resp[1] = iolink_checksum_ck(startup_resp[0], 0U);
    assert_int_equal(iolink_master_on_rx(&port, startup_resp, sizeof(startup_resp)),
                     IOLINK_MASTER_STATUS_OK);
    assert_int_equal(iolink_master_get_timing(&port, &timing), IOLINK_MASTER_STATUS_OK);
    assert_false(timing.awaiting_response);
}

int main(void)
{
    const struct CMUnitTest tests[] = {
        cmocka_unit_test_setup(test_public_api_drives_startup_and_latches_process_data,
                               reset_fixture),
        cmocka_unit_test_setup(test_public_api_exposes_scheduler_timing_state,
                               reset_fixture),
    };

    return cmocka_run_group_tests(tests, NULL, NULL);
}
