#include <setjmp.h>
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>
#include <limits.h>

#include <cmocka.h>

#include "iolinki/crc.h"
#include "test_wire_helpers.h"
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

/** @brief Queue the CKS-only DeviceOperate reply (A.1.6 over [0x00] = 0x2D). */
static void queue_operate_ack(void)
{
    static const uint8_t ack[1] = {0x2DU};

    queue_bytes(ack, sizeof(ack));
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
    startup_resp[1] = test_ck6_type0(startup_resp[0]);
    queue_bytes(startup_resp, sizeof(startup_resp));

    assert_int_equal(iolink_master_tick(&port, false), 1);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_PREOPERATE);
    assert_int_equal(g_send_calls, 3);
    assert_int_equal(g_sent_len[2], 3U);
    assert_int_equal(g_sent[2][0],
                     iolink_master_encode_master_command(false, IOLINK_MASTER_MC_CHANNEL_PAGE,
                                                         IOLINK_MASTER_DPP1_OFF_MASTER_COMMAND));
    assert_int_equal(g_sent[2][IOLINK_M_SEQ_HEADER_LEN], IOLINK_CMD_DEVICE_OPERATE);

    /* Figure A.5: the DeviceOperate write is answered with the CKS octet alone;
       a non-transmitting tick consumes it before the port enters OPERATE. */
    queue_operate_ack();
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);
}

static void test_tick_applies_timeout_before_transmit(void** state)
{
    iolink_master_port_t port;

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_phy, &g_config), 0);
    iolink_master_port_state(&port)->state = IOLINK_MASTER_STATE_OPERATE;
    iolink_master_port_state(&port)->diagnostics.rx_retry_count = 2U;

    /* 7.2.2.1: retry exhaustion restarts communication rather than latching ERROR. */
    assert_int_equal(iolink_master_tick(&port, true), 1);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_STARTUP);
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
    startup_resp[1] = test_ck6_type0(startup_resp[0]);
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
    startup_resp[1] = test_ck6_type0(startup_resp[0]);
    queue_bytes(startup_resp, sizeof(startup_resp));

    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 1);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_PREOPERATE);
    assert_int_equal(g_send_calls, 3);
    assert_int_equal(g_sent_len[2], 3U);
    assert_int_equal(g_sent[2][0],
                     iolink_master_encode_master_command(false, IOLINK_MASTER_MC_CHANNEL_PAGE,
                                                         IOLINK_MASTER_DPP1_OFF_MASTER_COMMAND));
    assert_int_equal(g_sent[2][IOLINK_M_SEQ_HEADER_LEN], IOLINK_CMD_DEVICE_OPERATE);

    /* Figure A.5: consume the CKS-only DeviceOperate reply before OPERATE. */
    queue_operate_ack();
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);
}

static void test_tick_event_response_timeout_applies_before_transmit(void** state)
{
    iolink_master_port_t port;

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_phy, &g_config), 0);
    iolink_master_port_state(&port)->state = IOLINK_MASTER_STATE_OPERATE;
    iolink_master_port_state(&port)->diagnostics.rx_retry_count = 2U;

    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_RESPONSE_TIMEOUT), 1);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_STARTUP);
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
    iolink_master_config_t config = g_config;
    uint32_t next_due = 0U;

    (void)state;

    /* A response timeout equal to the min cycle time keeps the deadline aligned
       with the next cycle so this test isolates the cycle pacing (A.3.5). */
    config.response_timeout_100us = 20U;

    assert_int_equal(iolink_master_init(&port, &g_phy, &config), 0);
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

    /* One M-sequence at a time: while the reply is outstanding the due cycle
       is deferred, not pipelined. */
    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, 120U), 0);
    assert_int_equal(g_send_calls, 1);
    assert_int_equal(iolink_master_port_state(&port)->cycle_count, 1U);

    /* Consume the write reply ([PD-in] CKS, no OD octet), then the next due
       cycle proceeds. */
    {
        const uint8_t pd_in[1] = {0x00U};
        uint8_t reply[2];

        reply[0] = pd_in[0];
        reply[1] = test_ck6_reply(pd_in, 1U, 0U);
        queue_bytes(reply, sizeof(reply));
    }
    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_NONE, 120U), 1);
    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, 120U), 0);
    assert_int_equal(g_send_calls, 2);
    assert_int_equal(iolink_master_port_state(&port)->cycle_count, 2U);
}

static void test_response_timeout_flushes_partial_reply_bytes(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;
    iolink_master_diagnostics_t diagnostics;

    (void)state;

    config.response_timeout_100us = 20U;

    assert_int_equal(iolink_master_init(&port, &g_phy, &config), 0);
    iolink_master_port_state(&port)->state = IOLINK_MASTER_STATE_OPERATE;

    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, 100U), 0);
    assert_int_equal(g_send_calls, 1);

    /* One stale reply byte from the timed-out request is already inside the
       port's RX buffer (a poll consumed it before the deadline). */
    {
        const uint8_t partial[1] = {0x11U};

        queue_bytes(partial, sizeof(partial));
    }
    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_NONE, 110U), 0);

    /* The timeout drops the partial byte, then the retry reply frames cleanly:
       without the flush the stale byte would mis-frame it and raise a checksum
       error. */
    {
        int tr = iolink_master_tick_at(&port, IOLINK_MASTER_TICK_RESPONSE_TIMEOUT, 120U);

        assert_true(tr >= 0);
    }
    {
        const uint8_t pd_in[1] = {0x00U};
        uint8_t reply[2];

        reply[0] = pd_in[0];
        reply[1] = test_ck6_reply(pd_in, 1U, 0U);
        queue_bytes(reply, sizeof(reply));
    }
    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_NONE, 121U), 1);
    assert_int_equal(iolink_master_get_diagnostics(&port, &diagnostics), 0);
    assert_int_equal(diagnostics.checksum_errors, 0U);
    assert_int_equal(diagnostics.rx_retry_count, 0U);
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

static void test_tick_at_holds_first_message_for_t_dmt_after_wake(void** state)
{
    iolink_master_port_t port;
    iolink_master_tick_event_t event = IOLINK_MASTER_TICK_CYCLE_DUE;

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_phy, &g_config), 0);

    /* 7.3.2.2: the first test message follows the wake-up after T_REN (max
       500us, Table 10) and T_DMT (Table 42). Wake-up at t=0; COM3 T_DMT =
       32 * 4.34us = 139us -> 2 ticks; 5 + 2 = 7 ticks of 100us. */
    assert_int_equal(iolink_master_tick_at(&port, event, 0U), 0);
    assert_int_equal(g_send_calls, 1);
    assert_int_equal(iolink_master_port_state(&port)->startup.step,
                     IOLINK_MASTER_STARTUP_STEP_SEND_TYPE0);
    assert_int_equal(iolink_master_port_state(&port)->send_ready_at_100us, 7U);

    /* T_REN + T_DMT not yet elapsed: the test message is held off. */
    assert_int_equal(iolink_master_tick_at(&port, event, 6U), 0);
    assert_int_equal(g_send_calls, 1);
    assert_int_equal(iolink_master_port_state(&port)->startup.step,
                     IOLINK_MASTER_STARTUP_STEP_SEND_TYPE0);

    /* At t=700us the first test message is sent. */
    assert_int_equal(iolink_master_tick_at(&port, event, 7U), 0);
    assert_int_equal(g_send_calls, 2);
    assert_int_equal(iolink_master_port_state(&port)->startup.step,
                     IOLINK_MASTER_STARTUP_STEP_AWAIT_RESPONSE);
}

static void test_tick_at_spaces_wake_retries_by_t_dwu(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;
    iolink_master_tick_event_t cycle = IOLINK_MASTER_TICK_CYCLE_DUE;

    (void)state;

    config.wake_retry_limit = 1U;
    config.t_dwu_100us = 400U; /* 40 ms (Table 42). */

    assert_int_equal(iolink_master_init(&port, &g_phy, &config), 0);
    iolink_master_process(&port);
    assert_int_equal(g_send_calls, 1);

    /* The wake-up attempt fails; the retry is armed for now + T_DWU. */
    assert_int_equal(iolink_master_on_timeout_at(&port, 100U, true),
                     IOLINK_MASTER_STATUS_PENDING);
    assert_int_equal(iolink_master_port_state(&port)->send_ready_at_100us, 500U);

    /* Before T_DWU elapsed no new wake-up is transmitted. */
    assert_int_equal(iolink_master_tick_at(&port, cycle, 499U), 0);
    assert_int_equal(g_send_calls, 1);

    /* At T_DWU the wake-up retry goes out. */
    assert_int_equal(iolink_master_tick_at(&port, cycle, 500U), 0);
    assert_int_equal(g_send_calls, 2);
    assert_int_equal(iolink_master_port_state(&port)->startup.step,
                     IOLINK_MASTER_STARTUP_STEP_SEND_TYPE0);
}

static void test_response_deadline_has_a_t_bit_floor(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;

    (void)state;

    /* A.3.5/A.3.6: deadline >= one UART frame (11 T_BIT) + t_A (10 T_BIT). */
    config.response_timeout_100us = 0U;
    config.baudrate = IOLINK_BAUDRATE_COM3;
    assert_int_equal(iolink_master_init(&port, &g_phy, &config), 0);
    iolink_master_port_state(&port)->state = IOLINK_MASTER_STATE_OPERATE;
    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, 100U), 0);
    assert_int_equal(iolink_master_port_state(&port)->response_deadline_100us, 101U);

    reset_fixture(state);
    config.baudrate = IOLINK_BAUDRATE_COM1; /* 21 * 208.33us = 4375us -> 44 ticks. */
    assert_int_equal(iolink_master_init(&port, &g_phy, &config), 0);
    iolink_master_port_state(&port)->state = IOLINK_MASTER_STATE_OPERATE;
    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, 100U), 0);
    assert_int_equal(iolink_master_port_state(&port)->response_deadline_100us, 144U);
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
        cmocka_unit_test_setup(test_response_timeout_flushes_partial_reply_bytes,
                               reset_fixture),
        cmocka_unit_test_setup(test_next_tick_time_prefers_response_deadline_before_cycle_due,
                               reset_fixture),
        cmocka_unit_test_setup(test_response_timeout_config_sets_deadline_before_cycle_due,
                               reset_fixture),
        cmocka_unit_test_setup(test_tick_at_counts_late_cycle_slips, reset_fixture),
        cmocka_unit_test_setup(test_tick_at_tracks_cycle_jitter_diagnostics,
                               reset_fixture),
        cmocka_unit_test_setup(test_tick_at_holds_first_message_for_t_dmt_after_wake,
                               reset_fixture),
        cmocka_unit_test_setup(test_tick_at_spaces_wake_retries_by_t_dwu, reset_fixture),
        cmocka_unit_test_setup(test_response_deadline_has_a_t_bit_floor, reset_fixture),
        cmocka_unit_test_setup(test_tick_rejects_null_port, reset_fixture),
    };

    return cmocka_run_group_tests(tests, NULL, NULL);
}
