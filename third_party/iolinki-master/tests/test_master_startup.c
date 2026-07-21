#include <setjmp.h>
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>
#include <limits.h>

#include <cmocka.h>

#include "iolinki/crc.h"
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
static iolink_baudrate_t g_baudrate_history[8];
static char g_io_direction_log[24];
static uint8_t g_io_direction_log_len;
static uint8_t g_sent[8][64];
static size_t g_sent_len[8];

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
    assert_in_range(g_set_baudrate_calls, 0, 7);
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
    assert_in_range(g_checked_set_baudrate_calls, 0, 7);
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
    assert_in_range(g_send_calls, 0, 7);

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
    assert_int_equal(iolink_master_port_state(&port)->od_len, 2);
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
    config.m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_2_1;
    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), 0);
    assert_int_equal(iolink_master_port_state(&port)->od_len, IOLINK_OD_LEN_16BIT);

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

static void test_auto_baudrate_startup_timeout_scans_com3_com2_com1_then_errors(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;

    (void)state;

    config.auto_baudrate = true;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), 0);
    assert_int_equal(g_set_baudrate_calls, 1);
    assert_int_equal(g_baudrate_history[0], IOLINK_BAUDRATE_COM3);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_STARTUP);

    iolink_master_process(&port);
    assert_int_equal(iolink_master_port_state(&port)->startup.step, 1U);

    assert_int_equal(iolink_master_on_timeout(&port), 1);
    assert_int_equal(iolink_master_port_state(&port)->startup.step, 0U);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_STARTUP);
    assert_int_equal(g_set_baudrate_calls, 2);
    assert_int_equal(g_baudrate_history[1], IOLINK_BAUDRATE_COM2);

    assert_int_equal(iolink_master_on_timeout(&port), 1);
    assert_int_equal(g_set_baudrate_calls, 3);
    assert_int_equal(g_baudrate_history[2], IOLINK_BAUDRATE_COM1);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_STARTUP);

    assert_int_equal(iolink_master_on_timeout(&port), -2);
    assert_int_equal(g_set_baudrate_calls, 3);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_ERROR);
}

static void test_auto_baudrate_timeout_flushes_adapter_rx_before_baud_change(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;

    (void)state;

    config.auto_baudrate = true;
    config.flush_rx = fake_flush_rx;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), IOLINK_MASTER_STATUS_OK);
    assert_int_equal(g_flush_rx_calls, 1);

    iolink_master_process(&port);
    assert_int_equal(iolink_master_on_timeout(&port), IOLINK_MASTER_STATUS_PENDING);
    assert_int_equal(g_flush_rx_calls, 2);
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

static void test_fixed_baudrate_startup_timeout_enters_error(void** state)
{
    iolink_master_port_t port;

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &g_config), 0);
    assert_int_equal(iolink_master_on_timeout(&port), -2);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_ERROR);
    assert_int_equal(g_set_baudrate_calls, 1);
}

static void test_wake_retry_limit_reissues_wake_before_error_on_fixed_baud(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;

    (void)state;

    config.wake_retry_limit = 2U;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), 0);

    /* Each timeout re-arms the wake-up (step back to 0) at the same baudrate. */
    assert_int_equal(iolink_master_on_timeout(&port), IOLINK_MASTER_STATUS_PENDING);
    assert_int_equal(iolink_master_port_state(&port)->startup.step, 0U);
    assert_int_equal(iolink_master_port_state(&port)->startup.wake_attempts, 1U);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_STARTUP);

    assert_int_equal(iolink_master_on_timeout(&port), IOLINK_MASTER_STATUS_PENDING);
    assert_int_equal(iolink_master_port_state(&port)->startup.wake_attempts, 2U);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_STARTUP);

    /* Wake budget exhausted on a fixed baudrate: enter error, never re-baud. */
    assert_int_equal(iolink_master_on_timeout(&port), -2);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_ERROR);
    assert_int_equal(g_set_baudrate_calls, 1);
}

static void test_wake_retry_exhausts_per_baud_then_advances_scan(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;

    (void)state;

    config.auto_baudrate = true;
    config.wake_retry_limit = 1U;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &config), 0);
    assert_int_equal(g_baudrate_history[0], IOLINK_BAUDRATE_COM3);

    /* One wake retry at COM3 before the scan is allowed to advance. */
    assert_int_equal(iolink_master_on_timeout(&port), IOLINK_MASTER_STATUS_PENDING);
    assert_int_equal(iolink_master_port_state(&port)->startup.wake_attempts, 1U);
    assert_int_equal(g_set_baudrate_calls, 1);

    /* Budget spent: advance to COM2 and reset the per-baud wake counter. */
    assert_int_equal(iolink_master_on_timeout(&port), IOLINK_MASTER_STATUS_PENDING);
    assert_int_equal(g_set_baudrate_calls, 2);
    assert_int_equal(g_baudrate_history[1], IOLINK_BAUDRATE_COM2);
    assert_int_equal(iolink_master_port_state(&port)->startup.wake_attempts, 0U);
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
    startup_resp[1] = iolink_checksum_ck(startup_resp[0], 0U);
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
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);

    iolink_master_process(&port);
    expected_len = iolink_frame_encode_type1_cycle(pd_out,
                                                   sizeof(pd_out),
                                                   iolink_master_port_state(&port)->od_len,
                                                   expected,
                                                   sizeof(expected));
    assert_int_equal(expected_len, 7);
    assert_int_equal(g_send_calls, 4);
    assert_int_equal(g_sent_len[3], (size_t)expected_len);
    assert_memory_equal(g_sent[3], expected, (size_t)expected_len);
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
    uint8_t ctrl;
    uint8_t frame[2];

    for(i = 0U; i < len; i++)
    {
        ctrl = i;
        if(i == 0U)
        {
            ctrl |= IOLINK_ISDU_CTRL_START;
        }
        if(i == (uint8_t)(len - 1U))
        {
            ctrl |= IOLINK_ISDU_CTRL_LAST;
        }

        frame[0] = ctrl;
        frame[1] = iolink_checksum_ck(frame[0], 0U);
        assert_int_equal(iolink_master_on_rx(port, frame, sizeof(frame)), 0);

        frame[0] = data[i];
        frame[1] = iolink_checksum_ck(frame[0], 0U);
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
    startup_resp[1] = iolink_checksum_ck(startup_resp[0], 0U);
    assert_int_equal(iolink_master_on_rx(&port, startup_resp, sizeof(startup_resp)), 0);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_PREOPERATE);

    iolink_master_process(&port);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_PREOPERATE);
    assert_int_equal(g_send_calls, 2);

    iolink_master_process(&port);
    assert_int_equal(g_send_calls, 3);
    assert_int_equal(g_sent_len[2], 2U);
    assert_int_equal(g_sent[2][0], IOLINK_ISDU_CTRL_START);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_PREOPERATE);

    feed_preoperate_isdu_response_bytes(&port, page1, sizeof(page1));

    iolink_master_process(&port);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);
    assert_int_equal(iolink_master_get_device_info(&port, &info), 0);
    assert_int_equal(info.vendor_id, 0x1234U);
    assert_int_equal(info.device_id, 0x56789AU);
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
    g_recv_bytes[1] = iolink_checksum_ck(g_recv_bytes[0], 0U);
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
    startup_resp[1] = iolink_checksum_ck(startup_resp[0], 0U);
    assert_int_equal(iolink_master_on_rx(&port, startup_resp, sizeof(startup_resp)), 0);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_PREOPERATE);

    assert_int_equal(iolink_master_read_isdu(&port, 0x0002U, 0U, data, &len), 1);

    g_recv_bytes[0] = IOLINK_ISDU_CTRL_START;
    g_recv_bytes[1] = iolink_checksum_ck(g_recv_bytes[0], 0U);
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

static void test_startup_bad_type0_response_retries_before_error_state(void** state)
{
    iolink_master_port_t port;
    const uint8_t bad_resp[] = {0x00U, 0x00U};

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &g_config), 0);
    iolink_master_process(&port);
    iolink_master_process(&port);
    assert_int_equal(iolink_master_port_state(&port)->startup.step, 2U);

    assert_int_equal(iolink_master_on_rx(&port, bad_resp, sizeof(bad_resp)), -3);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_STARTUP);

    assert_int_equal(iolink_master_on_rx(&port, bad_resp, sizeof(bad_resp)), -3);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_STARTUP);

    assert_int_equal(iolink_master_on_rx(&port, bad_resp, sizeof(bad_resp)), -3);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_ERROR);
}

int main(void)
{
    const struct CMUnitTest tests[] = {
        cmocka_unit_test_setup(test_init_rejects_null_args, reset_fake_phy),
        cmocka_unit_test_setup(test_valid_init_sets_startup_state, reset_fake_phy),
        cmocka_unit_test_setup(test_validate_phy_contract_rejects_missing_hardware_ops,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_init_uses_checked_mode_and_baudrate_hooks,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_init_propagates_checked_mode_failure, reset_fake_phy),
        cmocka_unit_test_setup(test_init_propagates_checked_baudrate_failure,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_init_flushes_adapter_rx_before_iolink_startup,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_init_propagates_adapter_rx_flush_failure,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_init_sets_od_length_from_m_sequence_type, reset_fake_phy),
        cmocka_unit_test_setup(test_init_deactivated_port_sets_inactive_phy_and_does_not_send,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_init_di_and_dq_ports_stay_in_sio_and_do_not_send,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_set_dq_drives_cq_line_only_for_dq_ports,
                               reset_fake_phy),
        cmocka_unit_test_setup(
            test_auto_baudrate_startup_timeout_scans_com3_com2_com1_then_errors,
            reset_fake_phy),
        cmocka_unit_test_setup(
            test_auto_baudrate_timeout_flushes_adapter_rx_before_baud_change,
            reset_fake_phy),
        cmocka_unit_test_setup(
            test_auto_baudrate_timeout_propagates_adapter_rx_flush_failure,
            reset_fake_phy),
        cmocka_unit_test_setup(test_fixed_baudrate_startup_timeout_enters_error,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_wake_retry_limit_reissues_wake_before_error_on_fixed_baud,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_wake_retry_exhausts_per_baud_then_advances_scan,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_restart_reenters_startup_and_clears_runtime_state,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_get_diagnostics_samples_hardware_fault_hooks,
                               reset_fake_phy),
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
        cmocka_unit_test_setup(test_process_startup_uses_configured_wake_up_hook,
                               reset_fake_phy),
        cmocka_unit_test_setup(
            test_process_wraps_core_send_with_half_duplex_direction_hooks,
            reset_fake_phy),
        cmocka_unit_test_setup(test_process_reports_prepare_tx_failure_before_send,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_process_reports_prepare_rx_failure_after_send,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_process_startup_reports_failed_wake_up_hook,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_startup_can_validate_device_info_before_operate,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_poll_rx_accepts_startup_type0_response_from_phy,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_poll_rx_accepts_preoperate_type0_isdu_response_from_phy,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_process_partial_send_enters_error_state, reset_fake_phy),
        cmocka_unit_test_setup(test_startup_bad_type0_response_retries_before_error_state,
                               reset_fake_phy),
    };

    return cmocka_run_group_tests(tests, NULL, NULL);
}
