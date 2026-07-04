#include <setjmp.h>
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>
#include <limits.h>

#include <cmocka.h>

#include "iolinki/crc.h"
#include "iolinki/protocol.h"
#include "../src/master_internal.h"

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

static const iolink_master_config_t g_config = {
    .port_mode = IOLINK_MASTER_PORT_MODE_IOLINK,
    .m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_2_1,
    .baudrate = IOLINK_BAUDRATE_COM3,
    .min_cycle_time = 20U,
    .pd_in_len = 1U,
    .pd_out_len = 0U,
    .auto_baudrate = false,
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

static void test_tick_sends_startup_frame_when_no_rx(void** state)
{
    iolink_master_port_t port;

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_phy, &g_config), 0);
    assert_int_equal(iolink_master_tick(&port, false), 0);
    assert_int_equal(g_send_calls, 1);
    assert_int_equal(g_sent_len[0], 1U);
    assert_int_equal(g_sent[0][0], 0x55U);
}

static void test_tick_drains_rx_before_sending_next_frame(void** state)
{
    iolink_master_port_t port;
    uint8_t startup_resp[2] = {0U};

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_phy, &g_config), 0);
    iolink_master_process(&port);
    iolink_master_process(&port);
    assert_int_equal(iolink_master_port_state(&port)->startup.step, 2U);

    startup_resp[0] = 0x00U;
    startup_resp[1] = iolink_checksum_ck(startup_resp[0], 0U);
    queue_bytes(startup_resp, sizeof(startup_resp));

    assert_int_equal(iolink_master_tick(&port, false), 1);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);
    assert_int_equal(g_send_calls, 3);
    assert_int_equal(g_sent_len[2], 3U);
    assert_int_equal(g_sent[2][0],
                     iolink_master_encode_master_command(false, IOLINK_MASTER_MC_CHANNEL_PAGE,
                                                         IOLINK_MASTER_DPP1_OFF_MASTER_COMMAND));
    assert_int_equal(g_sent[2][1], IOLINK_CMD_DEVICE_OPERATE);
}

static void test_tick_applies_timeout_before_transmit(void** state)
{
    iolink_master_port_t port;

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_phy, &g_config), 0);
    iolink_master_port_state(&port)->state = IOLINK_MASTER_STATE_OPERATE;
    iolink_master_port_state(&port)->diagnostics.rx_retry_count = 2U;

    assert_int_equal(iolink_master_tick(&port, true), -2);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_ERROR);
    assert_int_equal(g_send_calls, 0);
}

static void test_tick_event_none_drains_rx_without_transmitting(void** state)
{
    iolink_master_port_t port;
    uint8_t startup_resp[2] = {0U};

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_phy, &g_config), 0);
    iolink_master_process(&port);
    iolink_master_process(&port);
    assert_int_equal(iolink_master_port_state(&port)->startup.step, 2U);

    startup_resp[0] = 0x00U;
    startup_resp[1] = iolink_checksum_ck(startup_resp[0], 0U);
    queue_bytes(startup_resp, sizeof(startup_resp));

    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_PREOPERATE);
    assert_int_equal(g_send_calls, 2);
}

static void test_tick_event_cycle_due_transmits_after_rx(void** state)
{
    iolink_master_port_t port;
    uint8_t startup_resp[2] = {0U};

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_phy, &g_config), 0);
    iolink_master_process(&port);
    iolink_master_process(&port);
    assert_int_equal(iolink_master_port_state(&port)->startup.step, 2U);

    startup_resp[0] = 0x00U;
    startup_resp[1] = iolink_checksum_ck(startup_resp[0], 0U);
    queue_bytes(startup_resp, sizeof(startup_resp));

    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 1);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);
    assert_int_equal(g_send_calls, 3);
    assert_int_equal(g_sent_len[2], 3U);
    assert_int_equal(g_sent[2][0],
                     iolink_master_encode_master_command(false, IOLINK_MASTER_MC_CHANNEL_PAGE,
                                                         IOLINK_MASTER_DPP1_OFF_MASTER_COMMAND));
    assert_int_equal(g_sent[2][1], IOLINK_CMD_DEVICE_OPERATE);
}

static void test_tick_event_response_timeout_applies_before_transmit(void** state)
{
    iolink_master_port_t port;

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_phy, &g_config), 0);
    iolink_master_port_state(&port)->state = IOLINK_MASTER_STATE_OPERATE;
    iolink_master_port_state(&port)->diagnostics.rx_retry_count = 2U;

    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_RESPONSE_TIMEOUT), -2);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_ERROR);
    assert_int_equal(g_send_calls, 0);
}

static void test_tick_event_response_timeout_reports_pending_retry(void** state)
{
    iolink_master_port_t port;

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_phy, &g_config), 0);
    iolink_master_port_state(&port)->state = IOLINK_MASTER_STATE_OPERATE;

    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_RESPONSE_TIMEOUT),
                     IOLINK_MASTER_STATUS_PENDING);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);
    assert_int_equal(iolink_master_port_state(&port)->diagnostics.response_timeouts, 1U);
    assert_int_equal(iolink_master_port_state(&port)->diagnostics.rx_retry_count, 1U);
    assert_int_equal(g_send_calls, 0);
}

static void test_tick_at_paces_operate_cycles_by_min_cycle_time(void** state)
{
    iolink_master_port_t port;
    uint32_t next_due = 0U;

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_phy, &g_config), 0);
    iolink_master_port_state(&port)->state = IOLINK_MASTER_STATE_OPERATE;

    assert_int_equal(iolink_master_get_next_tick_time(&port, 100U, &next_due), 0);
    assert_int_equal(next_due, 100U);

    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, 100U), 0);
    assert_int_equal(g_send_calls, 1);
    assert_int_equal(iolink_master_port_state(&port)->cycle_count, 1U);
    assert_int_equal(iolink_master_get_next_tick_time(&port, 101U, &next_due), 0);
    assert_int_equal(next_due, 120U);

    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, 119U), 0);
    assert_int_equal(g_send_calls, 1);
    assert_int_equal(iolink_master_port_state(&port)->cycle_count, 1U);

    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, 120U), 0);
    assert_int_equal(g_send_calls, 2);
    assert_int_equal(iolink_master_port_state(&port)->cycle_count, 2U);
}

static void test_next_tick_time_prefers_response_deadline_before_cycle_due(void** state)
{
    iolink_master_port_t port;
    uint32_t next_due = 0U;

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_phy, &g_config), 0);
    iolink_master_port_state(&port)->state = IOLINK_MASTER_STATE_OPERATE;
    iolink_master_port_state(&port)->cycle_timer_valid = true;
    iolink_master_port_state(&port)->last_cycle_start_100us = 100U;
    iolink_master_port_state(&port)->response_deadline_100us = 110U;
    iolink_master_port_state(&port)->awaiting_response = true;

    assert_int_equal(iolink_master_get_next_tick_time(&port, 101U, &next_due), 0);
    assert_int_equal(next_due, 110U);
    assert_int_equal(iolink_master_get_next_tick_time(&port, 111U, &next_due), 0);
    assert_int_equal(next_due, 111U);

    iolink_master_port_state(&port)->awaiting_response = false;
    assert_int_equal(iolink_master_get_next_tick_time(&port, 111U, &next_due), 0);
    assert_int_equal(next_due, 120U);
}

static void test_response_timeout_config_sets_deadline_before_cycle_due(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;
    uint32_t next_due = 0U;

    (void)state;

    config.response_timeout_100us = 3U;

    assert_int_equal(iolink_master_init(&port, &g_phy, &config), 0);
    iolink_master_port_state(&port)->state = IOLINK_MASTER_STATE_OPERATE;

    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, 100U), 0);
    assert_true(iolink_master_port_state(&port)->awaiting_response);
    assert_int_equal(iolink_master_port_state(&port)->response_deadline_100us, 103U);
    assert_int_equal(iolink_master_get_next_tick_time(&port, 101U, &next_due), 0);
    assert_int_equal(next_due, 103U);
}

static void test_tick_at_counts_late_cycle_slips(void** state)
{
    iolink_master_port_t port;
    iolink_master_diagnostics_t diagnostics;

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_phy, &g_config), 0);
    iolink_master_port_state(&port)->state = IOLINK_MASTER_STATE_OPERATE;

    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, 100U), 0);
    assert_int_equal(g_send_calls, 1);
    iolink_master_port_state(&port)->awaiting_response = false;

    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, 120U), 0);
    assert_int_equal(g_send_calls, 2);
    assert_int_equal(iolink_master_get_diagnostics(&port, &diagnostics), 0);
    assert_int_equal(diagnostics.cycle_slips, 0U);
    iolink_master_port_state(&port)->awaiting_response = false;

    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, 145U), 0);
    assert_int_equal(g_send_calls, 3);
    assert_int_equal(iolink_master_get_diagnostics(&port, &diagnostics), 0);
    assert_int_equal(diagnostics.cycle_slips, 1U);
}

static void test_tick_at_tracks_cycle_jitter_diagnostics(void** state)
{
    iolink_master_port_t port;
    iolink_master_diagnostics_t diagnostics;

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_phy, &g_config), 0);
    iolink_master_port_state(&port)->state = IOLINK_MASTER_STATE_OPERATE;

    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, 100U), 0);
    iolink_master_port_state(&port)->awaiting_response = false;

    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, 120U), 0);
    assert_int_equal(iolink_master_get_diagnostics(&port, &diagnostics), 0);
    assert_int_equal(diagnostics.last_cycle_jitter_100us, 0U);
    assert_int_equal(diagnostics.max_cycle_jitter_100us, 0U);
    iolink_master_port_state(&port)->awaiting_response = false;

    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, 145U), 0);
    assert_int_equal(iolink_master_get_diagnostics(&port, &diagnostics), 0);
    assert_int_equal(diagnostics.last_cycle_jitter_100us, 5U);
    assert_int_equal(diagnostics.max_cycle_jitter_100us, 5U);
    iolink_master_port_state(&port)->awaiting_response = false;

    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, 166U), 0);
    assert_int_equal(iolink_master_get_diagnostics(&port, &diagnostics), 0);
    assert_int_equal(diagnostics.last_cycle_jitter_100us, 1U);
    assert_int_equal(diagnostics.max_cycle_jitter_100us, 5U);
}

static void test_tick_rejects_null_port(void** state)
{
    (void)state;

    assert_int_equal(iolink_master_tick(NULL, false), -1);
    assert_int_equal(iolink_master_tick_event(NULL, IOLINK_MASTER_TICK_CYCLE_DUE), -1);
    assert_int_equal(iolink_master_tick_at(NULL, IOLINK_MASTER_TICK_CYCLE_DUE, 0U), -1);
}

int main(void)
{
    const struct CMUnitTest tests[] = {
        cmocka_unit_test_setup(test_tick_sends_startup_frame_when_no_rx, reset_fixture),
        cmocka_unit_test_setup(test_tick_drains_rx_before_sending_next_frame, reset_fixture),
        cmocka_unit_test_setup(test_tick_applies_timeout_before_transmit, reset_fixture),
        cmocka_unit_test_setup(test_tick_event_none_drains_rx_without_transmitting,
                               reset_fixture),
        cmocka_unit_test_setup(test_tick_event_cycle_due_transmits_after_rx, reset_fixture),
        cmocka_unit_test_setup(test_tick_event_response_timeout_applies_before_transmit,
                               reset_fixture),
        cmocka_unit_test_setup(test_tick_event_response_timeout_reports_pending_retry,
                               reset_fixture),
        cmocka_unit_test_setup(test_tick_at_paces_operate_cycles_by_min_cycle_time,
                               reset_fixture),
        cmocka_unit_test_setup(test_next_tick_time_prefers_response_deadline_before_cycle_due,
                               reset_fixture),
        cmocka_unit_test_setup(test_response_timeout_config_sets_deadline_before_cycle_due,
                               reset_fixture),
        cmocka_unit_test_setup(test_tick_at_counts_late_cycle_slips, reset_fixture),
        cmocka_unit_test_setup(test_tick_at_tracks_cycle_jitter_diagnostics,
                               reset_fixture),
        cmocka_unit_test_setup(test_tick_rejects_null_port, reset_fixture),
    };

    return cmocka_run_group_tests(tests, NULL, NULL);
}
