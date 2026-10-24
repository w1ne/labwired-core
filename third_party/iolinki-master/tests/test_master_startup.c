#include <setjmp.h>
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>
#include <limits.h>

#include <cmocka.h>

#include "iolinki/crc.h"
#include "test_wire_helpers.h"
#include "iolinki/frame.h"
#include "iolinki/protocol.h"
#include "../src/master_internal.h"

static int g_init_calls;
static int g_set_mode_calls;
static int g_set_baudrate_calls;
static int g_checked_set_mode_calls;
static int g_checked_set_baudrate_calls;
static int g_checked_set_mode_result;
static int g_checked_set_baudrate_result;
static int g_flush_rx_calls;
static int g_flush_rx_result;
static int g_prepare_tx_calls;
static int g_prepare_rx_calls;
static int g_prepare_tx_result;
static int g_prepare_rx_result;
static int g_send_calls;
static int g_set_cq_line_calls;
static int g_wake_up_calls;
static int g_wake_up_result;
static int g_voltage_mv;
static bool g_short_circuit;
static int g_forced_send_return;
static uint8_t g_recv_bytes[8];
static uint8_t g_recv_len;
static uint8_t g_recv_pos;
static uint8_t g_last_cq_line;
static iolink_phy_mode_t g_last_mode;
static iolink_baudrate_t g_last_baudrate;
static iolink_baudrate_t g_baudrate_history[16];
static char g_io_direction_log[24];
static uint8_t g_io_direction_log_len;
static uint8_t g_sent[9][64];
static size_t g_sent_len[9];

static int fake_phy_init(void* user)
{
    (void)user;
    g_init_calls++;
    return 0;
}

static void fake_phy_set_mode(void* user, iolink_phy_mode_t mode)
{
    (void)user;
    g_set_mode_calls++;
    g_last_mode = mode;
}

static void fake_phy_set_baudrate(void* user, iolink_baudrate_t baudrate)
{
    (void)user;
    assert_in_range(g_set_baudrate_calls, 0, 15);
    g_baudrate_history[g_set_baudrate_calls] = baudrate;
    g_set_baudrate_calls++;
    g_last_baudrate = baudrate;
}

static int fake_checked_set_mode(iolink_phy_mode_t mode)
{
    g_checked_set_mode_calls++;
    g_last_mode = mode;
    return g_checked_set_mode_result;
}

static int fake_checked_set_baudrate(iolink_baudrate_t baudrate)
{
    assert_in_range(g_checked_set_baudrate_calls, 0, 15);
    g_baudrate_history[g_checked_set_baudrate_calls] = baudrate;
    g_checked_set_baudrate_calls++;
    g_last_baudrate = baudrate;
    return g_checked_set_baudrate_result;
}

static int fake_flush_rx(void)
{
    g_flush_rx_calls++;
    return g_flush_rx_result;
}

static void append_io_direction_log(char event)
{
    assert_in_range(g_io_direction_log_len, 0, sizeof(g_io_direction_log) - 1U);
    g_io_direction_log[g_io_direction_log_len++] = event;
    g_io_direction_log[g_io_direction_log_len] = '\0';
}

static int fake_prepare_tx(void)
{
    g_prepare_tx_calls++;
    append_io_direction_log('T');
    return g_prepare_tx_result;
}

static int fake_prepare_rx(void)
{
    g_prepare_rx_calls++;
    append_io_direction_log('R');
    return g_prepare_rx_result;
}

static void fake_phy_set_cq_line(void* user, uint8_t state)
{
    (void)user;
    g_set_cq_line_calls++;
    g_last_cq_line = state;
}

static int fake_wake_up(void)
{
    g_wake_up_calls++;
    return g_wake_up_result;
}

static int fake_phy_get_voltage_mv(void* user)
{
    (void)user;
    return g_voltage_mv;
}

static bool fake_phy_is_short_circuit(void* user)
{
    (void)user;
    return g_short_circuit;
}

static int fake_phy_send(void* user, const uint8_t* data, size_t len)
{
    (void)user;
    assert_non_null(data);
    assert_in_range(len, 1U, sizeof(g_sent[0]));
    assert_in_range(g_send_calls, 0, 8);

    memcpy(g_sent[g_send_calls], data, len);
    g_sent_len[g_send_calls] = len;
    g_send_calls++;
    append_io_direction_log('S');

    if(g_forced_send_return != INT_MIN)
    {
        return g_forced_send_return;
    }

    return (int)len;
}

static int fake_phy_recv_byte(void* user, uint8_t* byte)
{
    (void)user;
    assert_non_null(byte);

    if(g_recv_pos >= g_recv_len)
    {
        return 0;
    }

    *byte = g_recv_bytes[g_recv_pos++];
    return 1;
}

static const iolink_phy_api_t g_fake_phy = {
    .init = fake_phy_init,
    .set_mode = fake_phy_set_mode,
    .set_baudrate = fake_phy_set_baudrate,
    .send = fake_phy_send,
    .recv_byte = fake_phy_recv_byte,
    .set_cq_line = fake_phy_set_cq_line,
    .get_voltage_mv = fake_phy_get_voltage_mv,
    .is_short_circuit = fake_phy_is_short_circuit,
};

static const iolink_master_config_t g_config = {
    .m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_2_1,
    .baudrate = IOLINK_BAUDRATE_COM3,
    .min_cycle_time = 20U,
    .pd_in_len = 4U,
    .pd_out_len = 2U,
};

static int reset_fake_phy(void** state)
{
    (void)state;
    g_init_calls = 0;
    g_set_mode_calls = 0;
    g_set_baudrate_calls = 0;
    g_checked_set_mode_calls = 0;
    g_checked_set_baudrate_calls = 0;
    g_checked_set_mode_result = IOLINK_MASTER_STATUS_OK;
    g_checked_set_baudrate_result = IOLINK_MASTER_STATUS_OK;
    g_flush_rx_calls = 0;
    g_flush_rx_result = IOLINK_MASTER_STATUS_OK;
    g_prepare_tx_calls = 0;
    g_prepare_rx_calls = 0;
    g_prepare_tx_result = IOLINK_MASTER_STATUS_OK;
    g_prepare_rx_result = IOLINK_MASTER_STATUS_OK;
    g_send_calls = 0;
    g_set_cq_line_calls = 0;
    g_wake_up_calls = 0;
    g_wake_up_result = IOLINK_MASTER_STATUS_OK;
    g_voltage_mv = 24100;
    g_short_circuit = false;
    g_forced_send_return = INT_MIN;
    g_recv_len = 0U;
    g_recv_pos = 0U;
    g_last_cq_line = 0U;
    g_last_mode = IOLINK_PHY_MODE_INACTIVE;
    g_last_baudrate = IOLINK_BAUDRATE_COM1;
    g_io_direction_log_len = 0U;
    memset(g_io_direction_log, 0, sizeof(g_io_direction_log));
    memset(g_baudrate_history, 0, sizeof(g_baudrate_history));
    memset(g_sent, 0, sizeof(g_sent));
    memset(g_sent_len, 0, sizeof(g_sent_len));
    return 0;
}

static void test_init_rejects_null_args(void** state)
{
    iolink_master_port_t port;

    (void)state;

    assert_int_equal(iolink_master_init(NULL, &g_fake_phy, &g_config), -1);
    assert_int_equal(iolink_master_init(&port, NULL, &g_config), -1);
    assert_int_equal(iolink_master_init(&port, &g_fake_phy, NULL), -1);
}

static void test_valid_init_sets_startup_state(void** state)
{
    iolink_master_port_t port;

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &g_config), 0);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_STARTUP);
    /* g_config is TYPE_2_1: one OD octet per Table A.10. */
    assert_int_equal(iolink_master_port_state(&port)->od_len, IOLINK_OD_LEN_8BIT);
    assert_int_equal(iolink_master_port_state(&port)->pd_in_len, g_config.pd_in_len);
    assert_int_equal(g_init_calls, 1);
    assert_int_equal(g_set_baudrate_calls, 1);
    assert_int_equal(g_last_baudrate, IOLINK_BAUDRATE_COM3);
    assert_int_equal(g_set_mode_calls, 1);
    assert_int_equal(g_last_mode, IOLINK_PHY_MODE_SDCI);
}

static void test_validate_phy_contract_rejects_missing_hardware_ops(void** state)
{
    iolink_master_config_t config = g_config;
    iolink_phy_api_t phy = g_fake_phy;

    (void)state;

    config.wake_up = fake_wake_up;
    config.set_mode_checked = fake_checked_set_mode;
    config.set_baudrate_checked = fake_checked_set_baudrate;
    config.flush_rx = fake_flush_rx;
    config.prepare_tx = fake_prepare_tx;
    config.prepare_rx = fake_prepare_rx;
    assert_int_equal(iolink_master_validate_phy_contract(&phy, &config),
                     IOLINK_MASTER_STATUS_OK);

    config.wake_up = NULL;
    assert_int_equal(iolink_master_validate_phy_contract(&phy, &config),
                     IOLINK_MASTER_ERR_UNSUPPORTED_PHY);

    config.wake_up = fake_wake_up;
    phy.recv_byte = NULL;
    assert_int_equal(iolink_master_validate_phy_contract(&phy, &config),
                     IOLINK_MASTER_ERR_UNSUPPORTED_PHY);

    phy = g_fake_phy;
    config.set_baudrate_checked = NULL;
    assert_int_equal(iolink_master_validate_phy_contract(&phy, &config),
                     IOLINK_MASTER_ERR_UNSUPPORTED_PHY);

    config.set_baudrate_checked = fake_checked_set_baudrate;
    config.set_mode_checked = NULL;
    assert_int_equal(iolink_master_validate_phy_contract(&phy, &config),
                     IOLINK_MASTER_ERR_UNSUPPORTED_PHY);

    config.set_mode_checked = fake_checked_set_mode;
    config.flush_rx = NULL;
    assert_int_equal(iolink_master_validate_phy_contract(&phy, &config),
                     IOLINK_MASTER_ERR_UNSUPPORTED_PHY);

    config.flush_rx = fake_flush_rx;
    config.prepare_tx = NULL;
    assert_int_equal(iolink_master_validate_phy_contract(&phy, &config),
                     IOLINK_MASTER_ERR_UNSUPPORTED_PHY);

    config.prepare_tx = fake_prepare_tx;
    config.prepare_rx = NULL;
    assert_int_equal(iolink_master_validate_phy_contract(&phy, &config),
                     IOLINK_MASTER_ERR_UNSUPPORTED_PHY);

    config.prepare_rx = fake_prepare_rx;
    config.port_mode = IOLINK_MASTER_PORT_MODE_DQ;
    phy = g_fake_phy;
    phy.set_cq_line = NULL;
    assert_int_equal(iolink_master_validate_phy_contract(&phy, &config),
                     IOLINK_MASTER_ERR_UNSUPPORTED_PHY);

    config.port_mode = IOLINK_MASTER_PORT_MODE_DI;
    config.read_cq_line_checked = NULL;
    assert_int_equal(iolink_master_validate_phy_contract(&g_fake_phy, &config),
                     IOLINK_MASTER_ERR_UNSUPPORTED_PHY);
}

static void test_init_uses_checked_mode_and_baudrate_hooks(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;

    (void)state;

    config.set_mode_checked = fake_checked_set_mode;
    config.set_baudrate_checked = fake_checked_set_baudrate;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), IOLINK_MASTER_STATUS_OK);
    assert_int_equal(g_checked_set_baudrate_calls, 1);
    assert_int_equal(g_checked_set_mode_calls, 1);
    assert_int_equal(g_set_baudrate_calls, 0);
    assert_int_equal(g_set_mode_calls, 0);
    assert_int_equal(g_last_baudrate, IOLINK_BAUDRATE_COM3);
    assert_int_equal(g_last_mode, IOLINK_PHY_MODE_SDCI);
}

static void test_init_propagates_checked_mode_failure(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;

    (void)state;

    config.set_mode_checked = fake_checked_set_mode;
    config.set_baudrate_checked = fake_checked_set_baudrate;
    g_checked_set_mode_result = IOLINK_MASTER_ERR_UNSUPPORTED_PHY;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config),
                     IOLINK_MASTER_ERR_UNSUPPORTED_PHY);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_ERROR);
    assert_int_equal(g_checked_set_baudrate_calls, 1);
    assert_int_equal(g_checked_set_mode_calls, 1);
}

static void test_init_propagates_checked_baudrate_failure(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;

    (void)state;

    config.set_mode_checked = fake_checked_set_mode;
    config.set_baudrate_checked = fake_checked_set_baudrate;
    g_checked_set_baudrate_result = IOLINK_MASTER_ERR_UNSUPPORTED_PHY;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config),
                     IOLINK_MASTER_ERR_UNSUPPORTED_PHY);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_ERROR);
    assert_int_equal(g_checked_set_baudrate_calls, 1);
    assert_int_equal(g_checked_set_mode_calls, 0);
}

static void test_init_flushes_adapter_rx_before_iolink_startup(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;

    (void)state;

    config.flush_rx = fake_flush_rx;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), IOLINK_MASTER_STATUS_OK);
    assert_int_equal(g_flush_rx_calls, 1);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_STARTUP);
}

static void test_init_propagates_adapter_rx_flush_failure(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;

    (void)state;

    config.flush_rx = fake_flush_rx;
    g_flush_rx_result = IOLINK_MASTER_ERR_FRAME;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config),
                     IOLINK_MASTER_ERR_FRAME);
    assert_int_equal(g_flush_rx_calls, 1);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_ERROR);
    assert_int_equal(g_set_baudrate_calls, 0);
    assert_int_equal(g_set_mode_calls, 0);
}

static void test_init_sets_od_length_from_m_sequence_type(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;

    (void)state;

    config.m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_1_1;
    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), 0);
    assert_int_equal(iolink_master_port_state(&port)->od_len, IOLINK_OD_LEN_8BIT);

    reset_fake_phy(state);
    /* Table A.10: TYPE_2_1 carries one OD octet; only TYPE_2_V carries two. */
    config.m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_2_1;
    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), 0);
    assert_int_equal(iolink_master_port_state(&port)->od_len, IOLINK_OD_LEN_8BIT);

    reset_fake_phy(state);
    config.m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_2_V;
    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), 0);
    assert_int_equal(iolink_master_port_state(&port)->od_len, IOLINK_OD_LEN_16BIT);
}

static void test_init_deactivated_port_sets_inactive_phy_and_does_not_send(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;

    (void)state;

    config.port_mode = IOLINK_MASTER_PORT_MODE_DEACTIVATED;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), 0);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_INACTIVE);
    assert_int_equal(g_set_baudrate_calls, 0);
    assert_int_equal(g_set_mode_calls, 1);
    assert_int_equal(g_last_mode, IOLINK_PHY_MODE_INACTIVE);

    iolink_master_process(&port);
    assert_int_equal(g_send_calls, 0);
}

static void test_init_di_and_dq_ports_stay_in_sio_and_do_not_send(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;

    (void)state;

    config.port_mode = IOLINK_MASTER_PORT_MODE_DI;
    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), 0);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_INACTIVE);
    assert_int_equal(g_set_baudrate_calls, 0);
    assert_int_equal(g_set_mode_calls, 1);
    assert_int_equal(g_last_mode, IOLINK_PHY_MODE_SIO);

    iolink_master_process(&port);
    assert_int_equal(g_send_calls, 0);

    reset_fake_phy(state);
    config.port_mode = IOLINK_MASTER_PORT_MODE_DQ;
    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), 0);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_INACTIVE);
    assert_int_equal(g_set_baudrate_calls, 0);
    assert_int_equal(g_set_mode_calls, 1);
    assert_int_equal(g_last_mode, IOLINK_PHY_MODE_SIO);

    iolink_master_process(&port);
    assert_int_equal(g_send_calls, 0);
}

static void test_set_dq_drives_cq_line_only_for_dq_ports(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;
    iolink_phy_api_t phy = g_fake_phy;

    (void)state;

    assert_int_equal(iolink_master_set_dq(NULL, true), -1);

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &g_config), 0);
    assert_int_equal(iolink_master_set_dq(&port, true), -2);

    reset_fake_phy(state);
    config.port_mode = IOLINK_MASTER_PORT_MODE_DI;
    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), 0);
    assert_int_equal(iolink_master_set_dq(&port, true), -2);

    reset_fake_phy(state);
    config.port_mode = IOLINK_MASTER_PORT_MODE_DQ;
    phy.set_cq_line = NULL;
    assert_int_equal(iolink_master_init(&port, &phy, &config), 0);
    assert_int_equal(iolink_master_set_dq(&port, true), -3);

    reset_fake_phy(state);
    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), 0);
    assert_int_equal(iolink_master_set_dq(&port, true), 0);
    assert_int_equal(g_set_cq_line_calls, 1);
    assert_int_equal(g_last_cq_line, 1U);

    assert_int_equal(iolink_master_set_dq(&port, false), 0);
    assert_int_equal(g_set_cq_line_calls, 2);
    assert_int_equal(g_last_cq_line, 0U);
    assert_int_equal(g_send_calls, 0);
}

static void test_auto_baudrate_scans_rates_per_wake_then_reports_failed_sequence(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;
    iolink_master_diagnostics_t diagnostics;
    uint8_t retry;

    (void)state;

    config.auto_baudrate = true;
    config.wake_up = fake_wake_up;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), 0);
    assert_int_equal(g_set_baudrate_calls, 1);
    assert_int_equal(g_baudrate_history[0], IOLINK_BAUDRATE_COM3);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_STARTUP);

    /*
     * 7.3.2.2 / Figure 36: each wake-up request is followed by one test message
     * at COM3, COM2 and COM1 (T15..T17); a silent last rate increments Retry
     * (T18) and the next wake-up resets the scan to COM3. n_WU = 2 (Table 42)
     * gives n_WU + 1 = 3 wake-up requests per sequence.
     */
    for (retry = 0U; retry < 3U; retry++) {
        iolink_master_process(&port); /* WURQ */
        assert_int_equal(g_wake_up_calls, retry + 1);
        iolink_master_process(&port); /* COM3 test message */
        assert_int_equal(g_last_baudrate, IOLINK_BAUDRATE_COM3);

        assert_int_equal(iolink_master_on_timeout(&port), IOLINK_MASTER_STATUS_PENDING);
        assert_int_equal(g_last_baudrate, IOLINK_BAUDRATE_COM2);
        assert_int_equal(iolink_master_port_state(&port)->startup.step,
                         IOLINK_MASTER_STARTUP_STEP_SEND_TYPE0);
        iolink_master_process(&port); /* COM2 test message, no new WURQ */
        assert_int_equal(g_wake_up_calls, retry + 1);

        assert_int_equal(iolink_master_on_timeout(&port), IOLINK_MASTER_STATUS_PENDING);
        assert_int_equal(g_last_baudrate, IOLINK_BAUDRATE_COM1);
        iolink_master_process(&port); /* COM1 test message */

        if (retry < 2U) {
            assert_int_equal(iolink_master_on_timeout(&port), IOLINK_MASTER_STATUS_PENDING);
            assert_int_equal(iolink_master_port_state(&port)->startup.step,
                             IOLINK_MASTER_STARTUP_STEP_WAKE);
            assert_int_equal(iolink_master_port_state(&port)->startup.wake_attempts, retry + 1);
            assert_int_equal(g_last_baudrate, IOLINK_BAUDRATE_COM3);
        }
    }

    /* Retry = 3 (T5): the sequence failed. It is reported, the PHY goes inactive
       (7.3.2.2) and the port stays in STARTUP instead of latching ERROR. */
    assert_int_equal(iolink_master_on_timeout(&port), IOLINK_MASTER_ERR_RETRY_LIMIT);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_STARTUP);
    assert_int_equal(iolink_master_port_state(&port)->startup.step,
                     IOLINK_MASTER_STARTUP_STEP_INACTIVE);
    assert_int_equal(g_last_mode, IOLINK_PHY_MODE_INACTIVE);
    assert_int_equal(g_send_calls, 9); /* 3 WURQs x 3 test messages */
    assert_int_equal(iolink_master_get_diagnostics(&port, &diagnostics), 0);
    assert_int_equal(diagnostics.establish_failures, 1U);

    /* Figure 33: a new wake-up retry sequence follows (after T_SD on a clock). */
    iolink_master_process(&port);
    assert_int_equal(g_last_mode, IOLINK_PHY_MODE_SDCI);
    assert_int_equal(g_wake_up_calls, 4);
    assert_int_equal(iolink_master_port_state(&port)->startup.step,
                     IOLINK_MASTER_STARTUP_STEP_SEND_TYPE0);
}

static void test_auto_baudrate_timeout_flushes_adapter_rx_before_baud_change(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;

    (void)state;

    config.auto_baudrate = true;
    config.flush_rx = fake_flush_rx;
    config.wake_retry_limit = 2U;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), IOLINK_MASTER_STATUS_OK);
    assert_int_equal(g_flush_rx_calls, 1);

    iolink_master_process(&port);
    assert_int_equal(iolink_master_on_timeout(&port), IOLINK_MASTER_STATUS_PENDING);
    assert_int_equal(iolink_master_on_timeout(&port), IOLINK_MASTER_STATUS_PENDING);
    assert_int_equal(iolink_master_on_timeout(&port), IOLINK_MASTER_STATUS_PENDING);
    assert_int_equal(g_flush_rx_calls, 4);
    assert_int_equal(g_baudrate_history[1], IOLINK_BAUDRATE_COM2);
}

static void test_auto_baudrate_timeout_propagates_adapter_rx_flush_failure(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;

    (void)state;

    config.auto_baudrate = true;
    config.flush_rx = fake_flush_rx;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), IOLINK_MASTER_STATUS_OK);
    g_flush_rx_result = IOLINK_MASTER_ERR_FRAME;

    iolink_master_process(&port);
    assert_int_equal(iolink_master_on_timeout(&port), IOLINK_MASTER_ERR_FRAME);
    assert_int_equal(g_flush_rx_calls, 2);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_ERROR);
    assert_int_equal(g_set_baudrate_calls, 1);
}

static void test_fixed_baudrate_failed_sequence_restarts_instead_of_error(void** state)
{
    iolink_master_port_t port;

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &g_config), 0);
    /* Table 42: n_WU = 2 wake-up retries, i.e. 3 wake-up requests per sequence. */
    assert_int_equal(iolink_master_on_timeout(&port), IOLINK_MASTER_STATUS_PENDING);
    assert_int_equal(iolink_master_on_timeout(&port), IOLINK_MASTER_STATUS_PENDING);
    assert_int_equal(iolink_master_on_timeout(&port), IOLINK_MASTER_ERR_RETRY_LIMIT);
    /* 7.3.2.2 / Figure 33: report the failure, PHY inactive, keep trying. */
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_STARTUP);
    assert_int_equal(g_last_mode, IOLINK_PHY_MODE_INACTIVE);
    assert_int_equal(g_set_baudrate_calls, 1);
}

static void test_wake_retry_limit_counts_wake_ups_per_sequence_on_fixed_baud(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;

    (void)state;

    config.wake_retry_limit = 2U;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), 0);

    /* Each failed attempt re-arms the wake-up at the same (only) rate. */
    assert_int_equal(iolink_master_on_timeout(&port), IOLINK_MASTER_STATUS_PENDING);
    assert_int_equal(iolink_master_port_state(&port)->startup.step, 0U);
    assert_int_equal(iolink_master_port_state(&port)->startup.wake_attempts, 1U);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_STARTUP);

    assert_int_equal(iolink_master_on_timeout(&port), IOLINK_MASTER_STATUS_PENDING);
    assert_int_equal(iolink_master_port_state(&port)->startup.wake_attempts, 2U);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_STARTUP);

    /* Retry = n_WU + 1: the sequence ends; the counter restarts for the next. */
    assert_int_equal(iolink_master_on_timeout(&port), IOLINK_MASTER_ERR_RETRY_LIMIT);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_STARTUP);
    assert_int_equal(iolink_master_port_state(&port)->startup.wake_attempts, 0U);
    assert_int_equal(g_set_baudrate_calls, 1);

    /* No test message is in flight while the port waits T_SD. */
    assert_int_equal(iolink_master_on_timeout(&port), IOLINK_MASTER_STATUS_PENDING);
    assert_int_equal(iolink_master_port_state(&port)->startup.step,
                     IOLINK_MASTER_STARTUP_STEP_INACTIVE);
}

static void test_auto_baudrate_next_rate_does_not_repeat_wake_up(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;

    (void)state;

    config.auto_baudrate = true;
    config.wake_up = fake_wake_up;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), 0);
    assert_int_equal(g_baudrate_history[0], IOLINK_BAUDRATE_COM3);
    iolink_master_process(&port);
    iolink_master_process(&port);
    assert_int_equal(g_wake_up_calls, 1);

    /* Figure 36 T16: no answer at COM3 -> COM2 test message, same wake-up. */
    assert_int_equal(iolink_master_on_timeout(&port), IOLINK_MASTER_STATUS_PENDING);
    assert_int_equal(g_set_baudrate_calls, 2);
    assert_int_equal(g_baudrate_history[1], IOLINK_BAUDRATE_COM2);
    assert_int_equal(iolink_master_port_state(&port)->startup.wake_attempts, 0U);
    iolink_master_process(&port);
    assert_int_equal(g_wake_up_calls, 1);
    assert_int_equal(g_send_calls, 2);
    assert_int_equal(g_sent[1][0], 0xA2U);
}

static void test_restart_reenters_startup_and_clears_runtime_state(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;

    (void)state;

    config.auto_baudrate = true;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), 0);
    iolink_master_port_state(&port)->state = IOLINK_MASTER_STATE_ERROR;
    iolink_master_port_state(&port)->startup.step = 2U;
    iolink_master_port_state(&port)->startup.baudrate_index = 2U;
    iolink_master_port_state(&port)->diagnostics.rx_retry_count = 2U;
    iolink_master_port_state(&port)->diagnostics.checksum_errors = 5U;
    iolink_master_port_state(&port)->diagnostics.send_errors = 3U;
    iolink_master_port_state(&port)->diagnostics.response_timeouts = 4U;
    iolink_master_port_state(&port)->diagnostics.cycle_slips = 6U;
    iolink_master_port_state(&port)->diagnostics.last_cycle_jitter_100us = 7U;
    iolink_master_port_state(&port)->diagnostics.max_cycle_jitter_100us = 8U;
    iolink_master_port_state(&port)->diagnostics.link_quality_percent = 55U;
    iolink_master_port_state(&port)->diagnostics.last_service_result = -5;
    iolink_master_port_state(&port)->diagnostics.last_event_count = 2U;
    iolink_master_port_state(&port)->diagnostics.last_event_code = 0x1803U;
    iolink_master_port_state(&port)->diagnostics.last_isdu_error = 9U;
    iolink_master_port_state(&port)->cycle_count = 11U;

    assert_int_equal(iolink_master_restart(&port), 0);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_STARTUP);
    assert_int_equal(iolink_master_port_state(&port)->startup.step, 0U);
    assert_int_equal(iolink_master_port_state(&port)->startup.baudrate_index, 0U);
    assert_int_equal(iolink_master_port_state(&port)->diagnostics.rx_retry_count, 0U);
    assert_int_equal(iolink_master_port_state(&port)->diagnostics.checksum_errors, 0U);
    assert_int_equal(iolink_master_port_state(&port)->diagnostics.send_errors, 0U);
    assert_int_equal(iolink_master_port_state(&port)->diagnostics.response_timeouts, 0U);
    assert_int_equal(iolink_master_port_state(&port)->diagnostics.cycle_slips, 0U);
    assert_int_equal(iolink_master_port_state(&port)->diagnostics.last_cycle_jitter_100us, 0U);
    assert_int_equal(iolink_master_port_state(&port)->diagnostics.max_cycle_jitter_100us, 0U);
    assert_int_equal(iolink_master_port_state(&port)->diagnostics.link_quality_percent, 0U);
    assert_int_equal(iolink_master_port_state(&port)->diagnostics.last_service_result, 0);
    assert_int_equal(iolink_master_port_state(&port)->diagnostics.last_event_count, 0U);
    assert_int_equal(iolink_master_port_state(&port)->diagnostics.last_event_code, 0U);
    assert_int_equal(iolink_master_port_state(&port)->diagnostics.last_isdu_error, 0U);
    assert_int_equal(iolink_master_port_state(&port)->cycle_count, 0U);
    assert_int_equal(g_set_baudrate_calls, 2);
    assert_int_equal(g_baudrate_history[1], IOLINK_BAUDRATE_COM3);
    assert_int_equal(g_last_mode, IOLINK_PHY_MODE_SDCI);
}

static void test_get_diagnostics_samples_hardware_fault_hooks(void** state)
{
    iolink_master_port_t port;
    iolink_master_diagnostics_t diagnostics;

    (void)state;

    g_voltage_mv = 23600;
    g_short_circuit = true;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &g_config), 0);
    assert_int_equal(iolink_master_get_diagnostics(&port, &diagnostics), 0);
    assert_int_equal(diagnostics.supply_voltage_mv, 23600);
    assert_true(diagnostics.short_circuit);
}

static void test_restart_rejects_invalid_args(void** state)
{
    (void)state;

    assert_int_equal(iolink_master_restart(NULL), -1);
}

static void test_init_rejects_oversized_pd_in_len(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;

    (void)state;

    config.pd_in_len = (uint8_t)(IOLINK_PD_IN_MAX_SIZE + 1U);

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), -1);
    assert_int_equal(g_init_calls, 0);
}

static void test_init_rejects_oversized_pd_out_len(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;

    (void)state;

    config.pd_out_len = (uint8_t)(IOLINK_PD_OUT_MAX_SIZE + 1U);

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), -1);
    assert_int_equal(g_init_calls, 0);
}

static void test_init_rejects_invalid_baudrate_and_m_sequence_type(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;

    (void)state;

    config.baudrate = (iolink_baudrate_t)3U;
    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), -1);
    assert_int_equal(g_init_calls, 0);

    config = g_config;
    config.m_seq_type = (iolink_master_m_seq_type_t)7U;
    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), -1);
    assert_int_equal(g_init_calls, 0);
}

static void test_init_rejects_type0_with_process_data(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;

    (void)state;

    config.m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_0;
    config.pd_in_len = 1U;
    config.pd_out_len = 0U;
    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), -1);
    assert_int_equal(g_init_calls, 0);

    config = g_config;
    config.m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_0;
    config.pd_in_len = 0U;
    config.pd_out_len = 1U;
    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), -1);
    assert_int_equal(g_init_calls, 0);
}

static void test_get_pd_in_too_small_exposes_required_length(void** state)
{
    iolink_master_port_t port;
    uint8_t buffer[2] = {0U, 0U};
    uint8_t out_len = 0U;

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &g_config), 0);
    assert_int_equal(iolink_master_get_pd_in(&port, buffer, sizeof(buffer), &out_len), -2);
    assert_int_equal(out_len, g_config.pd_in_len);
}

static void test_get_pd_in_invalid_does_not_copy_stale_data(void** state)
{
    iolink_master_port_t port;
    uint8_t buffer[4] = {0xAAU, 0xAAU, 0xAAU, 0xAAU};
    uint8_t out_len = 0U;

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &g_config), 0);
    iolink_master_port_state(&port)->pd_in[0] = 0x11U;
    iolink_master_port_state(&port)->pd_in[1] = 0x22U;
    iolink_master_port_state(&port)->pd_in[2] = 0x33U;
    iolink_master_port_state(&port)->pd_in[3] = 0x44U;
    iolink_master_port_state(&port)->pd_valid = false;

    assert_int_equal(iolink_master_get_pd_in(&port, buffer, sizeof(buffer), &out_len), 1);
    assert_int_equal(out_len, g_config.pd_in_len);
    assert_int_equal(buffer[0], 0xAAU);
    assert_int_equal(buffer[1], 0xAAU);
    assert_int_equal(buffer[2], 0xAAU);
    assert_int_equal(buffer[3], 0xAAU);
}

static void test_process_startup_waits_for_type0_response_before_preoperate(void** state)
{
    iolink_master_port_t port;
    const uint8_t pd_out[] = {0x11U, 0x22U};
    uint8_t expected[8] = {0U};
    uint8_t startup_resp[2] = {0U};
    int expected_len;

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &g_config), 0);
    assert_int_equal(iolink_master_set_pd_out(&port, pd_out, sizeof(pd_out)), 0);

    iolink_master_process(&port);
    assert_int_equal(g_send_calls, 1);
    assert_int_equal(g_sent_len[0], 1U);
    assert_int_equal(g_sent[0][0], 0x55U);

    iolink_master_process(&port);
    /* Startup probe: Type-0 READ of MinCycleTime on the page channel (MC 0xA2). */
    expected_len = iolink_frame_encode_type0(
        iolink_master_encode_master_command(true, IOLINK_MASTER_MC_CHANNEL_PAGE,
                                            IOLINK_MASTER_DPP1_OFF_MIN_CYCLE_TIME),
        expected, sizeof(expected));
    assert_int_equal(expected_len, 2);
    assert_int_equal(g_send_calls, 2);
    assert_int_equal(g_sent_len[1], (size_t)expected_len);
    assert_memory_equal(g_sent[1], expected, (size_t)expected_len);
    assert_int_equal(iolink_master_port_state(&port)->startup.step, 2U);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_STARTUP);

    startup_resp[0] = 0x00U;
    startup_resp[1] = test_ck6_type0(startup_resp[0]);
    assert_int_equal(iolink_master_on_rx(&port, startup_resp, sizeof(startup_resp)), 0);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_PREOPERATE);

    iolink_master_process(&port);
    /* Transition to OPERATE: Type-0 WRITE of MasterCommand DeviceOperate (0x99)
       to Direct Parameter address 0x00 on the page channel (MC 0x20). */
    expected_len = iolink_frame_encode_type0_write(
        iolink_master_encode_master_command(false, IOLINK_MASTER_MC_CHANNEL_PAGE,
                                            IOLINK_MASTER_DPP1_OFF_MASTER_COMMAND),
        IOLINK_CMD_DEVICE_OPERATE, expected, sizeof(expected));
    assert_int_equal(expected_len, 3);
    assert_int_equal(g_send_calls, 3);
    assert_int_equal(g_sent_len[2], (size_t)expected_len);
    assert_memory_equal(g_sent[2], expected, (size_t)expected_len);
    /* Figure A.5: the write is answered by the CKS alone (oracle over [0x00] = 0x2D);
       OPERATE is entered only once it is verified. */
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_PREOPERATE);
    {
        const uint8_t operate_ack[1] = {0x2DU};
        assert_int_equal(iolink_master_on_rx(&port, operate_ack, 1U), 0);
    }
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);

    iolink_master_process(&port);
    /* A.2.4: the TYPE_2 cyclic message is MC, CKT, PD-out, OD with the A.1.6
       checksum and the M-sequence type in CKT (TYPE_2 -> 0x80). TYPE_2_1 carries
       one OD octet (Table A.10); A.1.6 folds the CKT with its type bits in
       place, and the oracle over [00 80 11 22 00] gives 0x05 -> CKT 0x85. No
       trailing checksum octet. */
    {
        const uint8_t expected_cycle[] = {0x00U, 0x85U, 0x11U, 0x22U, 0x00U};

        expected_len = (int) sizeof(expected_cycle);
        assert_int_equal(g_send_calls, 4);
        assert_int_equal(g_sent_len[3], sizeof(expected_cycle));
        assert_memory_equal(g_sent[3], expected_cycle, sizeof(expected_cycle));
    }
    assert_int_equal(iolink_master_port_state(&port)->cycle_count, 1U);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);
    assert_true(g_send_calls >= 4);
}

static void test_process_startup_uses_configured_wake_up_hook(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;

    (void)state;

    config.wake_up = fake_wake_up;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), 0);
    iolink_master_process(&port);

    assert_int_equal(g_wake_up_calls, 1);
    assert_int_equal(g_send_calls, 0);
    assert_int_equal(iolink_master_port_state(&port)->startup.step, 1U);
}

static void test_process_wraps_core_send_with_half_duplex_direction_hooks(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;

    (void)state;

    config.prepare_tx = fake_prepare_tx;
    config.prepare_rx = fake_prepare_rx;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), 0);
    iolink_master_process(&port);

    assert_int_equal(g_prepare_tx_calls, 1);
    assert_int_equal(g_prepare_rx_calls, 1);
    assert_int_equal(g_send_calls, 1);
    assert_string_equal(g_io_direction_log, "TSR");
    assert_int_equal(iolink_master_port_state(&port)->startup.step, 1U);
}

static void test_process_reports_prepare_tx_failure_before_send(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;

    (void)state;

    config.prepare_tx = fake_prepare_tx;
    config.prepare_rx = fake_prepare_rx;
    g_prepare_tx_result = IOLINK_MASTER_ERR_SERVICE;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), 0);
    iolink_master_process(&port);

    assert_int_equal(g_prepare_tx_calls, 1);
    assert_int_equal(g_prepare_rx_calls, 0);
    assert_int_equal(g_send_calls, 0);
    assert_string_equal(g_io_direction_log, "T");
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_ERROR);
    assert_int_equal(iolink_master_port_state(&port)->diagnostics.send_errors, 1U);
}

static void test_process_reports_prepare_rx_failure_after_send(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;

    (void)state;

    config.prepare_tx = fake_prepare_tx;
    config.prepare_rx = fake_prepare_rx;
    g_prepare_rx_result = IOLINK_MASTER_ERR_SERVICE;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), 0);
    iolink_master_process(&port);

    assert_int_equal(g_prepare_tx_calls, 1);
    assert_int_equal(g_prepare_rx_calls, 1);
    assert_int_equal(g_send_calls, 1);
    assert_string_equal(g_io_direction_log, "TSR");
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_ERROR);
    assert_int_equal(iolink_master_port_state(&port)->diagnostics.send_errors, 1U);
}

static void test_process_startup_reports_failed_wake_up_hook(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;

    (void)state;

    config.wake_up = fake_wake_up;
    g_wake_up_result = IOLINK_MASTER_ERR_SERVICE;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), 0);
    iolink_master_process(&port);

    assert_int_equal(g_wake_up_calls, 1);
    assert_int_equal(g_send_calls, 0);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_ERROR);
    assert_int_equal(iolink_master_port_state(&port)->diagnostics.send_errors, 1U);
}

static void feed_preoperate_isdu_response_bytes(iolink_master_port_t* port,
                                                const uint8_t* data,
                                                uint8_t len)
{
    uint8_t i;
    uint8_t frame[2];

    /* A.1.5 TYPE_0 reply: one OD octet plus CKS; the ISDU response stream is
       delivered one octet per read M-sequence (7.3.6.2). */
    for(i = 0U; i < len; i++)
    {
        frame[0] = data[i];
        frame[1] = test_ck6_type0(frame[0]);
        assert_int_equal(iolink_master_on_rx(port, frame, sizeof(frame)), 0);
    }
}

static void test_startup_can_validate_device_info_before_operate(void** state)
{
    static const uint8_t page1[] = {
        0x00U,
        0x00U,
        10U,
        0x01U,
        0x11U,
        0x83U,
        0x10U,
        0x12U,
        0x34U,
        0x56U,
        0x78U,
        0x9AU,
        0x00U,
        0x00U,
        0x00U,
        0x00U,
    };
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;
    iolink_master_device_info_t info;
    uint8_t startup_resp[2] = {0U};

    (void)state;

    config.validate_device_info = true;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), 0);
    iolink_master_process(&port);
    iolink_master_process(&port);
    startup_resp[1] = test_ck6_type0(startup_resp[0]);
    assert_int_equal(iolink_master_on_rx(&port, startup_resp, sizeof(startup_resp)), 0);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_PREOPERATE);

    /* Drive the ISDU request (index 0x0000) onto the ISDU channel until the
       transport waits for the response, then deliver the Read Response (+).
       TYPE_2_1 carries one OD octet per message (Table A.10), so the request is
       segmented into one message per ISDU octet. */
    for(uint8_t guard = 0U; guard < 8U; guard++)
    {
        iolink_master_process(&port);
        if(iolink_master_port_state(&port)->isdu.phase == IOLINK_MASTER_ISDU_PHASE_WAIT)
        {
            break;
        }
    }
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_PREOPERATE);
    assert_int_equal(iolink_master_port_state(&port)->isdu.phase,
                     IOLINK_MASTER_ISDU_PHASE_WAIT);

    {
        uint8_t response[24] = {0U};
        uint8_t chk = 0U;
        uint8_t i;

        /* Read Response (+), Length = 1, ExtLength = 2 + 16 + 1 = 19 (A.5.3). */
        response[0] = 0xD1U;
        response[1] = 19U;
        memcpy(&response[2], page1, sizeof(page1));
        for(i = 0U; i < (uint8_t)(2U + sizeof(page1)); i++)
        {
            chk ^= response[i];
        }
        response[(uint8_t)(2U + sizeof(page1))] = chk;

        feed_preoperate_isdu_response_bytes(&port, response, 19U);
    }

    for(uint8_t guard = 0U; guard < 16U; guard++)
    {
        iolink_master_process(&port);
        if(iolink_master_port_state(&port)->startup.step ==
           IOLINK_MASTER_STARTUP_STEP_AWAIT_OPERATE_ACK)
        {
            /* Figure A.5: the DeviceOperate write is answered with the CKS octet
               alone; consume it here so the loop leaves the AWAIT step. */
            static const uint8_t operate_ack[1] = {0x2DU};
            assert_int_equal(iolink_master_on_rx(&port, operate_ack, sizeof(operate_ack)), 0);
        }
        if(iolink_master_get_state(&port) == IOLINK_MASTER_STATE_OPERATE)
        {
            break;
        }
    }

    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);
    assert_int_equal(iolink_master_get_device_info(&port, &info), 0);
    assert_int_equal(info.vendor_id, 0x1234U);
    assert_int_equal(info.device_id, 0x56789AU);
}

static void test_startup_probe_octet_stored_under_no_check(void** state)
{
    iolink_master_port_t port;
    uint8_t startup_resp[2] = {0U};
    const uint8_t probe = 0x2AU;

    (void)state;

    /* NO_CHECK must still latch the MinCycleTime probe octet (Table B.3). */
    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &g_config), 0);
    assert_int_equal(iolink_master_port_state(&port)->config.inspection_level,
                     IOLINK_MASTER_INSPECTION_NO_CHECK);
    iolink_master_process(&port); /* WURQ */
    iolink_master_process(&port); /* test message */

    startup_resp[0] = probe;
    startup_resp[1] = test_ck6_type0(startup_resp[0]);
    assert_int_equal(iolink_master_on_rx(&port, startup_resp, sizeof(startup_resp)), 0);
    assert_int_equal(iolink_master_port_state(&port)->device_info.min_cycle_time, probe);
}

static void test_poll_rx_accepts_startup_type0_response_from_phy(void** state)
{
    iolink_master_port_t port;

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &g_config), 0);
    iolink_master_process(&port);
    iolink_master_process(&port);
    assert_int_equal(iolink_master_port_state(&port)->startup.step, 2U);

    g_recv_bytes[0] = 0x00U;
    g_recv_bytes[1] = test_ck6_type0(g_recv_bytes[0]);
    g_recv_len = 2U;
    g_recv_pos = 0U;

    assert_int_equal(iolink_master_poll_rx(&port), 1);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_PREOPERATE);
}

static void test_poll_rx_accepts_preoperate_type0_isdu_response_from_phy(void** state)
{
    iolink_master_port_t port;
    uint8_t startup_resp[2] = {0U};
    uint8_t data[8] = {0U};
    uint8_t len = sizeof(data);

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &g_config), 0);
    iolink_master_process(&port);
    iolink_master_process(&port);
    startup_resp[1] = test_ck6_type0(startup_resp[0]);
    assert_int_equal(iolink_master_on_rx(&port, startup_resp, sizeof(startup_resp)), 0);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_PREOPERATE);

    assert_int_equal(iolink_master_read_isdu(&port, 0x0002U, 0U, data, &len), 1);

    g_recv_bytes[0] = IOLINK_ISDU_CTRL_START;
    g_recv_bytes[1] = test_ck6_type0(g_recv_bytes[0]);
    g_recv_len = 2U;
    g_recv_pos = 0U;

    assert_int_equal(iolink_master_poll_rx(&port), 1);
}

static void test_process_partial_send_enters_error_state(void** state)
{
    iolink_master_port_t port;

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &g_config), 0);
    iolink_master_process(&port);
    assert_int_equal(iolink_master_port_state(&port)->startup.step, 1U);

    g_forced_send_return = 1;
    iolink_master_process(&port);

    assert_int_equal(g_send_calls, 2);
    assert_int_equal(iolink_master_port_state(&port)->startup.step, 1U);
    assert_int_equal(iolink_master_port_state(&port)->diagnostics.send_errors, 1U);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_ERROR);
}

static void test_startup_bad_type0_response_is_no_response(void** state)
{
    iolink_master_port_t port;
    const uint8_t bad_resp[] = {0x00U, 0x00U}; /* A.1.6: [0x00] needs CKS 0x2D */

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &g_config), 0);
    iolink_master_process(&port);
    iolink_master_process(&port);
    assert_int_equal(iolink_master_port_state(&port)->startup.step, 2U);

    /* Table 46 AwaitReply_1: an undecodable answer is no answer. The test
       message is not retried and the port never latches ERROR on it. */
    assert_int_equal(iolink_master_on_rx(&port, bad_resp, sizeof(bad_resp)), -3);
    assert_int_equal(iolink_master_on_rx(&port, bad_resp, sizeof(bad_resp)), -3);
    assert_int_equal(iolink_master_on_rx(&port, bad_resp, sizeof(bad_resp)), -3);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_STARTUP);
    assert_int_equal(iolink_master_port_state(&port)->diagnostics.checksum_errors, 3U);

    /* The attempt ends at its deadline and the establish sequence moves on. */
    assert_int_equal(iolink_master_on_timeout(&port), IOLINK_MASTER_STATUS_PENDING);
    assert_int_equal(iolink_master_port_state(&port)->startup.step,
                     IOLINK_MASTER_STARTUP_STEP_WAKE);
    assert_int_equal(iolink_master_port_state(&port)->startup.wake_attempts, 1U);
}

static void test_startup_reply_without_test_message_is_rejected(void** state)
{
    iolink_master_port_t port;
    const uint8_t resp[] = {0x00U, 0x2DU}; /* valid A.1.6 TYPE_0 reply */

    (void) state;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &g_config), 0);
    assert_int_equal(iolink_master_on_rx(&port, resp, sizeof(resp)), IOLINK_MASTER_ERR_FRAME);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_STARTUP);
}

int main(void)
{
    const struct CMUnitTest tests[] = {
        cmocka_unit_test_setup(test_init_rejects_null_args, reset_fake_phy),
        cmocka_unit_test_setup(test_valid_init_sets_startup_state, reset_fake_phy),
        cmocka_unit_test_setup(test_validate_phy_contract_rejects_missing_hardware_ops,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_init_uses_checked_mode_and_baudrate_hooks, reset_fake_phy),
        cmocka_unit_test_setup(test_init_propagates_checked_mode_failure, reset_fake_phy),
        cmocka_unit_test_setup(test_init_propagates_checked_baudrate_failure, reset_fake_phy),
        cmocka_unit_test_setup(test_init_flushes_adapter_rx_before_iolink_startup, reset_fake_phy),
        cmocka_unit_test_setup(test_init_propagates_adapter_rx_flush_failure, reset_fake_phy),
        cmocka_unit_test_setup(test_init_sets_od_length_from_m_sequence_type, reset_fake_phy),
        cmocka_unit_test_setup(test_init_deactivated_port_sets_inactive_phy_and_does_not_send,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_init_di_and_dq_ports_stay_in_sio_and_do_not_send,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_set_dq_drives_cq_line_only_for_dq_ports, reset_fake_phy),
        cmocka_unit_test_setup(test_auto_baudrate_scans_rates_per_wake_then_reports_failed_sequence,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_auto_baudrate_timeout_flushes_adapter_rx_before_baud_change,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_auto_baudrate_timeout_propagates_adapter_rx_flush_failure,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_fixed_baudrate_failed_sequence_restarts_instead_of_error,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_wake_retry_limit_counts_wake_ups_per_sequence_on_fixed_baud,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_auto_baudrate_next_rate_does_not_repeat_wake_up,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_restart_reenters_startup_and_clears_runtime_state,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_get_diagnostics_samples_hardware_fault_hooks, reset_fake_phy),
        cmocka_unit_test_setup(test_restart_rejects_invalid_args, reset_fake_phy),
        cmocka_unit_test_setup(test_init_rejects_oversized_pd_in_len, reset_fake_phy),
        cmocka_unit_test_setup(test_init_rejects_oversized_pd_out_len, reset_fake_phy),
        cmocka_unit_test_setup(test_init_rejects_invalid_baudrate_and_m_sequence_type,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_init_rejects_type0_with_process_data, reset_fake_phy),
        cmocka_unit_test_setup(test_get_pd_in_too_small_exposes_required_length, reset_fake_phy),
        cmocka_unit_test_setup(test_get_pd_in_invalid_does_not_copy_stale_data, reset_fake_phy),
        cmocka_unit_test_setup(test_process_startup_waits_for_type0_response_before_preoperate,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_process_startup_uses_configured_wake_up_hook, reset_fake_phy),
        cmocka_unit_test_setup(test_process_wraps_core_send_with_half_duplex_direction_hooks,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_process_reports_prepare_tx_failure_before_send, reset_fake_phy),
        cmocka_unit_test_setup(test_process_reports_prepare_rx_failure_after_send, reset_fake_phy),
        cmocka_unit_test_setup(test_process_startup_reports_failed_wake_up_hook, reset_fake_phy),
        cmocka_unit_test_setup(test_startup_can_validate_device_info_before_operate,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_startup_probe_octet_stored_under_no_check, reset_fake_phy),
        cmocka_unit_test_setup(test_poll_rx_accepts_startup_type0_response_from_phy,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_poll_rx_accepts_preoperate_type0_isdu_response_from_phy,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_process_partial_send_enters_error_state, reset_fake_phy),
        cmocka_unit_test_setup(test_startup_bad_type0_response_is_no_response, reset_fake_phy),
        cmocka_unit_test_setup(test_startup_reply_without_test_message_is_rejected, reset_fake_phy),
    };

    return cmocka_run_group_tests(tests, NULL, NULL);
}
