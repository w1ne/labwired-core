#include <setjmp.h>
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#include <cmocka.h>

#include "../src/master_internal.h"

static int g_send_calls[2];
static int g_active_phy;

static int fake_send0(const uint8_t* data, size_t len)
{
    assert_non_null(data);
    assert_true(len > 0U);
    g_active_phy = 0;
    g_send_calls[0]++;
    return (int)len;
}

static int fake_send1(const uint8_t* data, size_t len)
{
    assert_non_null(data);
    assert_true(len > 0U);
    g_active_phy = 1;
    g_send_calls[1]++;
    return (int)len;
}

static const iolink_phy_api_t g_phys[] = {
    {.send = fake_send0},
    {.send = fake_send1},
};

static const iolink_master_config_t g_configs[] = {
    {
        .port_mode = IOLINK_MASTER_PORT_MODE_IOLINK,
        .m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_2_1,
        .baudrate = IOLINK_BAUDRATE_COM3,
        .min_cycle_time = 20U,
        .pd_in_len = 0U,
        .pd_out_len = 0U,
        .auto_baudrate = false,
    },
    {
        .port_mode = IOLINK_MASTER_PORT_MODE_IOLINK,
        .m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_2_1,
        .baudrate = IOLINK_BAUDRATE_COM2,
        .min_cycle_time = 20U,
        .pd_in_len = 0U,
        .pd_out_len = 0U,
        .auto_baudrate = false,
    },
};

static int reset_fixture(void** state)
{
    (void)state;
    g_send_calls[0] = 0;
    g_send_calls[1] = 0;
    g_active_phy = -1;
    return 0;
}

static void test_controller_init_initializes_each_port(void** state)
{
    iolink_master_controller_t controller;
    iolink_master_port_t ports[2];

    (void)state;

    assert_int_equal(iolink_master_controller_init(&controller, ports, 2U, g_phys, g_configs), 0);
    assert_int_equal(iolink_master_controller_state(&controller)->port_count, 2U);
    assert_ptr_equal(iolink_master_controller_state(&controller)->ports, ports);
    assert_int_equal(iolink_master_get_state(&ports[0]), IOLINK_MASTER_STATE_STARTUP);
    assert_int_equal(iolink_master_get_state(&ports[1]), IOLINK_MASTER_STATE_STARTUP);
}

static void test_controller_tick_all_ticks_each_port(void** state)
{
    iolink_master_controller_t controller;
    iolink_master_port_t ports[2];
    bool timeouts[2] = {false, false};

    (void)state;

    assert_int_equal(iolink_master_controller_init(&controller, ports, 2U, g_phys, g_configs), 0);
    assert_int_equal(iolink_master_controller_tick(&controller, timeouts), 0);
    assert_int_equal(g_send_calls[0], 1);
    assert_int_equal(g_send_calls[1], 1);
}

static void test_controller_tick_reports_first_error_but_ticks_all_ports(void** state)
{
    iolink_master_controller_t controller;
    iolink_master_port_t ports[2];
    bool timeouts[2] = {true, false};

    (void)state;

    assert_int_equal(iolink_master_controller_init(&controller, ports, 2U, g_phys, g_configs), 0);
    iolink_master_port_state(&ports[0])->state = IOLINK_MASTER_STATE_OPERATE;
    iolink_master_port_state(&ports[0])->diagnostics.rx_retry_count = 2U;

    assert_int_equal(iolink_master_controller_tick(&controller, timeouts), -2);
    assert_int_equal(iolink_master_get_state(&ports[0]), IOLINK_MASTER_STATE_ERROR);
    assert_int_equal(g_send_calls[1], 1);
}

static void test_controller_tick_events_allow_independent_port_events(void** state)
{
    iolink_master_controller_t controller;
    iolink_master_port_t ports[2];
    iolink_master_tick_event_t events[2] = {
        IOLINK_MASTER_TICK_NONE,
        IOLINK_MASTER_TICK_CYCLE_DUE,
    };

    (void)state;

    assert_int_equal(iolink_master_controller_init(&controller, ports, 2U, g_phys, g_configs), 0);
    assert_int_equal(iolink_master_controller_tick_events(&controller, events), 0);
    assert_int_equal(g_send_calls[0], 0);
    assert_int_equal(g_send_calls[1], 1);
}

static void test_controller_tick_events_report_first_error_but_tick_all_ports(void** state)
{
    iolink_master_controller_t controller;
    iolink_master_port_t ports[2];
    iolink_master_tick_event_t events[2] = {
        IOLINK_MASTER_TICK_RESPONSE_TIMEOUT,
        IOLINK_MASTER_TICK_CYCLE_DUE,
    };

    (void)state;

    assert_int_equal(iolink_master_controller_init(&controller, ports, 2U, g_phys, g_configs), 0);
    iolink_master_port_state(&ports[0])->state = IOLINK_MASTER_STATE_OPERATE;
    iolink_master_port_state(&ports[0])->diagnostics.rx_retry_count = 2U;

    assert_int_equal(iolink_master_controller_tick_events(&controller, events), -2);
    assert_int_equal(iolink_master_get_state(&ports[0]), IOLINK_MASTER_STATE_ERROR);
    assert_int_equal(g_send_calls[1], 1);
}

static void test_controller_tick_at_paces_ports_by_each_cycle_time(void** state)
{
    iolink_master_controller_t controller;
    iolink_master_port_t ports[2];
    iolink_master_config_t configs[2];

    (void)state;

    configs[0] = g_configs[0];
    configs[1] = g_configs[1];
    configs[0].min_cycle_time = 20U;
    configs[1].min_cycle_time = 30U;

    assert_int_equal(iolink_master_controller_init(&controller, ports, 2U, g_phys, configs), 0);
    iolink_master_port_state(&ports[0])->state = IOLINK_MASTER_STATE_OPERATE;
    iolink_master_port_state(&ports[1])->state = IOLINK_MASTER_STATE_OPERATE;

    assert_int_equal(iolink_master_controller_tick_at(&controller, 100U), 0);
    assert_int_equal(g_send_calls[0], 1);
    assert_int_equal(g_send_calls[1], 1);
    iolink_master_port_state(&ports[0])->awaiting_response = false;
    iolink_master_port_state(&ports[1])->awaiting_response = false;

    assert_int_equal(iolink_master_controller_tick_at(&controller, 119U), 0);
    assert_int_equal(g_send_calls[0], 1);
    assert_int_equal(g_send_calls[1], 1);

    assert_int_equal(iolink_master_controller_tick_at(&controller, 120U), 0);
    assert_int_equal(g_send_calls[0], 2);
    assert_int_equal(g_send_calls[1], 1);
    iolink_master_port_state(&ports[0])->awaiting_response = false;

    assert_int_equal(iolink_master_controller_tick_at(&controller, 130U), 0);
    assert_int_equal(g_send_calls[0], 2);
    assert_int_equal(g_send_calls[1], 2);
}

static void test_controller_tick_at_times_out_missing_response_before_next_cycle(void** state)
{
    iolink_master_controller_t controller;
    iolink_master_port_t ports[1];
    iolink_master_config_t configs[1];
    iolink_master_diagnostics_t diagnostics;

    (void)state;

    configs[0] = g_configs[0];
    configs[0].min_cycle_time = 20U;

    assert_int_equal(iolink_master_controller_init(&controller, ports, 1U, g_phys, configs), 0);
    iolink_master_port_state(&ports[0])->state = IOLINK_MASTER_STATE_OPERATE;
    iolink_master_port_state(&ports[0])->diagnostics.rx_retry_count = 2U;

    assert_int_equal(iolink_master_controller_tick_at(&controller, 100U), 0);
    assert_int_equal(g_send_calls[0], 1);
    assert_int_equal(iolink_master_get_state(&ports[0]), IOLINK_MASTER_STATE_OPERATE);

    assert_int_equal(iolink_master_controller_tick_at(&controller, 119U), 0);
    assert_int_equal(g_send_calls[0], 1);
    assert_int_equal(iolink_master_get_state(&ports[0]), IOLINK_MASTER_STATE_OPERATE);

    assert_int_equal(iolink_master_controller_tick_at(&controller, 120U), -2);
    assert_int_equal(g_send_calls[0], 1);
    assert_int_equal(iolink_master_get_state(&ports[0]), IOLINK_MASTER_STATE_ERROR);
    assert_int_equal(iolink_master_get_diagnostics(&ports[0], &diagnostics), 0);
    assert_int_equal(diagnostics.response_timeouts, 1U);
}

static void test_controller_next_tick_time_returns_earliest_port_deadline(void** state)
{
    iolink_master_controller_t controller;
    iolink_master_port_t ports[2];
    iolink_master_config_t configs[2];
    uint32_t next_due = 0U;

    (void)state;

    configs[0] = g_configs[0];
    configs[1] = g_configs[1];
    configs[0].min_cycle_time = 20U;
    configs[1].min_cycle_time = 40U;

    assert_int_equal(iolink_master_controller_init(&controller, ports, 2U, g_phys, configs), 0);
    iolink_master_port_state(&ports[0])->state = IOLINK_MASTER_STATE_OPERATE;
    iolink_master_port_state(&ports[1])->state = IOLINK_MASTER_STATE_OPERATE;

    assert_int_equal(iolink_master_controller_tick_at(&controller, 100U), 0);
    assert_int_equal(iolink_master_controller_get_next_tick_time(&controller, 101U, &next_due),
                     0);
    assert_int_equal(next_due, 120U);

    iolink_master_port_state(&ports[0])->awaiting_response = true;
    iolink_master_port_state(&ports[0])->response_deadline_100us = 110U;
    assert_int_equal(iolink_master_controller_get_next_tick_time(&controller, 101U, &next_due),
                     0);
    assert_int_equal(next_due, 110U);
}

static void test_controller_exposes_public_port_accessors(void** state)
{
    iolink_master_controller_t controller;
    iolink_master_port_t ports[2];
    iolink_master_port_t* port = NULL;
    uint8_t count = 0U;

    (void)state;

    assert_int_equal(iolink_master_controller_init(&controller, ports, 2U, g_phys, g_configs), 0);

    assert_int_equal(iolink_master_controller_get_port_count(&controller, &count), 0);
    assert_int_equal(count, 2U);
    assert_int_equal(iolink_master_controller_get_port(&controller, 1U, &port), 0);
    assert_ptr_equal(port, &ports[1]);

    assert_int_equal(iolink_master_controller_get_port_count(NULL, &count), -1);
    assert_int_equal(iolink_master_controller_get_port_count(&controller, NULL), -1);
    assert_int_equal(iolink_master_controller_get_port(NULL, 0U, &port), -1);
    assert_int_equal(iolink_master_controller_get_port(&controller, 0U, NULL), -1);
    assert_int_equal(iolink_master_controller_get_port(&controller, 2U, &port), -1);
}

static void test_controller_rejects_invalid_args(void** state)
{
    iolink_master_controller_t controller;
    iolink_master_port_t ports[2];

    (void)state;

    assert_int_equal(iolink_master_controller_init(NULL, ports, 2U, g_phys, g_configs), -1);
    assert_int_equal(iolink_master_controller_init(&controller, NULL, 2U, g_phys, g_configs), -1);
    assert_int_equal(iolink_master_controller_init(&controller, ports, 0U, g_phys, g_configs), -1);
    assert_int_equal(iolink_master_controller_init(&controller, ports, 2U, NULL, g_configs), -1);
    assert_int_equal(iolink_master_controller_init(&controller, ports, 2U, g_phys, NULL), -1);
    assert_int_equal(iolink_master_controller_tick(NULL, NULL), -1);
    assert_int_equal(iolink_master_controller_tick_events(NULL, NULL), -1);
    assert_int_equal(iolink_master_controller_tick_at(NULL, 0U), -1);
}

int main(void)
{
    const struct CMUnitTest tests[] = {
        cmocka_unit_test_setup(test_controller_init_initializes_each_port, reset_fixture),
        cmocka_unit_test_setup(test_controller_tick_all_ticks_each_port, reset_fixture),
        cmocka_unit_test_setup(test_controller_tick_reports_first_error_but_ticks_all_ports,
                               reset_fixture),
        cmocka_unit_test_setup(test_controller_tick_events_allow_independent_port_events,
                               reset_fixture),
        cmocka_unit_test_setup(test_controller_tick_events_report_first_error_but_tick_all_ports,
                               reset_fixture),
        cmocka_unit_test_setup(test_controller_tick_at_paces_ports_by_each_cycle_time,
                               reset_fixture),
        cmocka_unit_test_setup(
            test_controller_tick_at_times_out_missing_response_before_next_cycle,
            reset_fixture),
        cmocka_unit_test_setup(test_controller_next_tick_time_returns_earliest_port_deadline,
                               reset_fixture),
        cmocka_unit_test_setup(test_controller_exposes_public_port_accessors, reset_fixture),
        cmocka_unit_test_setup(test_controller_rejects_invalid_args, reset_fixture),
    };

    return cmocka_run_group_tests(tests, NULL, NULL);
}
