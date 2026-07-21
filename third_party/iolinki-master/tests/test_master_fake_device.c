#include <setjmp.h>
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#include <cmocka.h>

#include "fake_iolink_device.h"
#include "iolinki/protocol.h"
#include "iolinki_master/master.h"

static const iolink_master_config_t g_config = {
    .port_mode = IOLINK_MASTER_PORT_MODE_IOLINK,
    .m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_1_1,
    .baudrate = IOLINK_BAUDRATE_COM3,
    .min_cycle_time = 20U,
    .pd_in_len = 1U,
    .pd_out_len = 0U,
    .auto_baudrate = false,
};

static uint32_t g_event_pending_calls;
static uint32_t g_event_dispatch_calls;
static iolink_master_event_t g_last_dispatched_event;

static void on_event_pending(void* user)
{
    (void)user;
    g_event_pending_calls++;
}

static void on_event(void* user, const iolink_master_event_t* event)
{
    (void)user;
    g_event_dispatch_calls++;
    g_last_dispatched_event = *event;
}

static int reset_fixture(void** state)
{
    (void)state;
    fake_iolink_device_reset(0xA5U, 1U, 1U);
    g_event_pending_calls = 0U;
    g_event_dispatch_calls = 0U;
    memset(&g_last_dispatched_event, 0, sizeof(g_last_dispatched_event));
    return 0;
}

static void test_fake_device_drives_startup_and_paced_pd_cycle(void** state)
{
    iolink_master_port_t port;
    uint8_t pd[1] = {0U};
    uint8_t len = 0U;

    (void)state;

    assert_int_equal(iolink_master_init(&port, fake_iolink_device_phy(), &g_config), 0);

    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(fake_iolink_device_wakeup_count(), 1U);

    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_PREOPERATE);

    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);
    assert_int_equal(fake_iolink_device_transition_count(), 1U);

    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, 100U), 0);
    assert_int_equal(fake_iolink_device_operate_cycle_count(), 1U);

    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_NONE, 101U), 1);
    assert_int_equal(iolink_master_get_pd_in(&port, pd, sizeof(pd), &len), 0);
    assert_int_equal(len, 1U);
    assert_int_equal(pd[0], 0xA5U);

    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, 119U), 0);
    assert_int_equal(fake_iolink_device_operate_cycle_count(), 1U);

    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, 120U), 0);
    assert_int_equal(fake_iolink_device_operate_cycle_count(), 2U);
}

static void test_fake_device_conformance_matrix_nominal_profiles(void** state)
{
    static const struct
    {
        iolink_master_m_seq_type_t m_seq_type;
        uint8_t pd_in_len;
        uint8_t pd_out_len;
        uint8_t od_len;
        uint8_t pd_value;
    } cases[] = {
        {IOLINK_MASTER_M_SEQ_TYPE_1_1, 1U, 0U, 1U, 0x11U},
        {IOLINK_MASTER_M_SEQ_TYPE_1_2, 2U, 1U, 1U, 0x22U},
        {IOLINK_MASTER_M_SEQ_TYPE_2_2, 2U, 2U, 2U, 0x33U},
        {IOLINK_MASTER_M_SEQ_TYPE_2_V, 4U, 3U, 2U, 0x44U},
    };
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;
    uint8_t pd[4] = {0U};
    uint8_t len = 0U;
    size_t i;
    uint8_t j;

    (void)state;

    for(i = 0U; i < (sizeof(cases) / sizeof(cases[0])); i++)
    {
        fake_iolink_device_reset(cases[i].pd_value, cases[i].pd_in_len, cases[i].od_len);
        config.m_seq_type = cases[i].m_seq_type;
        config.pd_in_len = cases[i].pd_in_len;
        config.pd_out_len = cases[i].pd_out_len;

        assert_int_equal(iolink_master_init(&port, fake_iolink_device_phy(), &config), 0);
        assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
        assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
        assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
        assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
        assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);

        assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, 100U), 0);
        assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_NONE, 101U), 1);
        assert_int_equal(iolink_master_get_pd_in(&port, pd, sizeof(pd), &len), 0);
        assert_int_equal(len, cases[i].pd_in_len);
        for(j = 0U; j < len; j++)
        {
            assert_int_equal(pd[j], cases[i].pd_value);
        }
    }
}

static void test_fake_device_can_inject_bad_operate_checksum(void** state)
{
    iolink_master_port_t port;
    iolink_master_diagnostics_t diagnostics;

    (void)state;

    assert_int_equal(iolink_master_init(&port, fake_iolink_device_phy(), &g_config), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);

    fake_iolink_device_corrupt_next_response_checksum();

    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE),
                     IOLINK_MASTER_ERR_CHECKSUM);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);
    assert_int_equal(iolink_master_get_diagnostics(&port, &diagnostics), 0);
    assert_int_equal(diagnostics.checksum_errors, 1U);
    assert_int_equal(diagnostics.rx_retry_count, 1U);
}

static void test_fake_device_can_drop_response_for_timeout_path(void** state)
{
    iolink_master_port_t port;
    iolink_master_diagnostics_t diagnostics;

    (void)state;

    assert_int_equal(iolink_master_init(&port, fake_iolink_device_phy(), &g_config), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);

    fake_iolink_device_drop_next_response();

    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, 100U), 0);
    assert_int_equal(fake_iolink_device_operate_cycle_count(), 1U);
    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_NONE, 101U), 0);
    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_RESPONSE_TIMEOUT, 120U),
                     IOLINK_MASTER_STATUS_PENDING);

    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);
    assert_int_equal(iolink_master_get_diagnostics(&port, &diagnostics), 0);
    assert_int_equal(diagnostics.response_timeouts, 1U);
    assert_int_equal(diagnostics.rx_retry_count, 1U);
}

static void test_fake_device_truncated_response_is_discarded_after_timeout(void** state)
{
    iolink_master_port_t port;
    iolink_master_diagnostics_t diagnostics;
    uint8_t pd[1] = {0U};
    uint8_t len = 0U;

    (void)state;

    assert_int_equal(iolink_master_init(&port, fake_iolink_device_phy(), &g_config), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);

    fake_iolink_device_truncate_next_response();

    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, 100U), 0);
    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_NONE, 101U), 0);
    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_RESPONSE_TIMEOUT, 120U),
                     IOLINK_MASTER_STATUS_PENDING);
    assert_int_equal(iolink_master_get_diagnostics(&port, &diagnostics), 0);
    assert_int_equal(diagnostics.response_timeouts, 1U);

    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, 140U), 0);
    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_NONE, 141U), 1);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);
    assert_int_equal(iolink_master_get_pd_in(&port, pd, sizeof(pd), &len), 0);
    assert_int_equal(len, 1U);
    assert_int_equal(pd[0], 0xA5U);
}

static void test_fake_device_exposes_event_pending_status(void** state)
{
    iolink_master_port_t port;
    iolink_master_diagnostics_t diagnostics;

    (void)state;

    fake_iolink_device_set_event_pending(true);

    assert_int_equal(iolink_master_init(&port, fake_iolink_device_phy(), &g_config), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);

    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);

    assert_int_equal(iolink_master_get_diagnostics(&port, &diagnostics), 0);
    assert_true(diagnostics.event_pending);
    assert_true((diagnostics.od_status & IOLINK_OD_STATUS_EVENT) != 0U);
}

static void test_fake_device_serves_event_details(void** state)
{
    iolink_master_port_t port;
    iolink_master_diagnostics_t diagnostics;
    iolink_master_event_t events[1];
    uint8_t count = 0U;
    const uint8_t details[] = {0xE2U, 0x42U, 0x10U};
    uint8_t i;

    (void)state;

    memset(events, 0, sizeof(events));
    fake_iolink_device_set_event_pending(true);
    fake_iolink_device_set_isdu_object(IOLINK_IDX_DETAILED_DEVICE_STATUS, 0U, details, sizeof(details));

    assert_int_equal(iolink_master_init(&port, fake_iolink_device_phy(), &g_config), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);

    assert_int_equal(iolink_master_read_event_details(&port, events, 1U, &count),
                     IOLINK_MASTER_STATUS_PENDING);

    for(i = 0U; i < 13U; i++)
    {
        assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
        assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
    }

    assert_int_equal(iolink_master_read_event_details(&port, events, 1U, &count),
                     IOLINK_MASTER_STATUS_OK);
    assert_int_equal(count, 1U);
    assert_int_equal(events[0].qualifier, 0xE2U);
    assert_int_equal(events[0].type, IOLINK_MASTER_EVENT_TYPE_WARNING);
    assert_int_equal(events[0].code, 0x4210U);
    assert_int_equal(iolink_master_get_diagnostics(&port, &diagnostics), 0);
    assert_int_equal(diagnostics.last_event_count, 1U);
    assert_int_equal(diagnostics.last_event_code, 0x4210U);
}

static void test_fake_device_ack_event_reads_event_code(void** state)
{
    iolink_master_port_t port;
    iolink_master_diagnostics_t diagnostics;
    uint16_t event_code = 0U;
    uint8_t i;

    (void)state;

    fake_iolink_device_set_event_pending(true);
    fake_iolink_device_set_event_code(0x1803U);

    assert_int_equal(iolink_master_init(&port, fake_iolink_device_phy(), &g_config), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);

    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
    assert_int_equal(iolink_master_get_diagnostics(&port, &diagnostics), 0);
    assert_true(diagnostics.event_pending);

    assert_int_equal(iolink_master_ack_event(&port, &event_code), IOLINK_MASTER_STATUS_PENDING);

    for(i = 0U; i < 11U; i++)
    {
        assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
        assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
    }

    assert_int_equal(iolink_master_ack_event(&port, &event_code), IOLINK_MASTER_STATUS_OK);
    assert_int_equal(event_code, 0x1803U);
    assert_int_equal(iolink_master_get_diagnostics(&port, &diagnostics), 0);
    assert_int_equal(diagnostics.last_event_code, 0x1803U);
}

static void test_fake_device_dispatches_event_pending_on_rising_edge(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;

    (void)state;

    config.event_pending_handler = on_event_pending;
    fake_iolink_device_set_event_pending(true);

    assert_int_equal(iolink_master_init(&port, fake_iolink_device_phy(), &config), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);

    /* The first operate response carrying the OD Event flag dispatches once. */
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
    assert_int_equal(g_event_pending_calls, 1U);

    /* The flag stays set on later cycles: dispatch is edge-triggered, not level. */
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
    assert_int_equal(g_event_pending_calls, 1U);
}

static void test_fake_device_dispatches_decoded_events_to_handler(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;
    iolink_master_event_t events[1];
    uint8_t count = 0U;
    const uint8_t details[] = {0xE2U, 0x42U, 0x10U};
    uint8_t i;

    (void)state;

    memset(events, 0, sizeof(events));
    config.event_handler = on_event;
    fake_iolink_device_set_event_pending(true);
    fake_iolink_device_set_isdu_object(IOLINK_IDX_DETAILED_DEVICE_STATUS, 0U, details,
                                       sizeof(details));

    assert_int_equal(iolink_master_init(&port, fake_iolink_device_phy(), &config), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);

    assert_int_equal(iolink_master_read_event_details(&port, events, 1U, &count),
                     IOLINK_MASTER_STATUS_PENDING);

    for(i = 0U; i < 13U; i++)
    {
        assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
        assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
    }

    assert_int_equal(iolink_master_read_event_details(&port, events, 1U, &count),
                     IOLINK_MASTER_STATUS_OK);
    assert_int_equal(count, 1U);
    assert_int_equal(g_event_dispatch_calls, 1U);
    assert_int_equal(g_last_dispatched_event.qualifier, 0xE2U);
    assert_int_equal(g_last_dispatched_event.type, IOLINK_MASTER_EVENT_TYPE_WARNING);
    assert_int_equal(g_last_dispatched_event.code, 0x4210U);
}

static void test_fake_device_serves_isdu_object_dictionary_read(void** state)
{
    iolink_master_port_t port;
    uint8_t data[8] = {0U};
    uint8_t len = sizeof(data);
    const uint8_t object_value[] = {0x4FU, 0x4BU};
    uint8_t i;

    (void)state;

    fake_iolink_device_set_isdu_object(0x0010U, 0U, object_value, sizeof(object_value));

    assert_int_equal(iolink_master_init(&port, fake_iolink_device_phy(), &g_config), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);

    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, data, &len),
                     IOLINK_MASTER_STATUS_PENDING);

    for(i = 0U; i < 11U; i++)
    {
        assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
        assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
    }

    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, data, &len),
                     IOLINK_MASTER_STATUS_OK);
    assert_int_equal(len, 2U);
    assert_int_equal(data[0], 0x4FU);
    assert_int_equal(data[1], 0x4BU);
}

static void test_fake_device_accepts_isdu_object_dictionary_write(void** state)
{
    iolink_master_port_t port;
    uint8_t data[8] = {0U};
    uint8_t len = sizeof(data);
    const uint8_t object_value[] = {0x4FU, 0x4BU};
    const uint8_t updated_value[] = {0x4EU, 0x57U};
    uint8_t i;

    (void)state;

    fake_iolink_device_set_isdu_object(0x0010U, 0U, object_value, sizeof(object_value));

    assert_int_equal(iolink_master_init(&port, fake_iolink_device_phy(), &g_config), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);

    assert_int_equal(iolink_master_write_isdu(&port, 0x0010U, 0U, updated_value, sizeof(updated_value)),
                     IOLINK_MASTER_STATUS_PENDING);

    for(i = 0U; i < 13U; i++)
    {
        assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
        assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
    }

    assert_int_equal(iolink_master_write_isdu(&port, 0x0010U, 0U, updated_value, sizeof(updated_value)),
                     IOLINK_MASTER_STATUS_OK);

    len = sizeof(data);
    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, data, &len),
                     IOLINK_MASTER_STATUS_PENDING);

    for(i = 0U; i < 11U; i++)
    {
        assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
        assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
    }

    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, data, &len),
                     IOLINK_MASTER_STATUS_OK);
    assert_int_equal(len, 2U);
    assert_int_equal(data[0], 0x4EU);
    assert_int_equal(data[1], 0x57U);
}

static void test_fake_device_verifies_written_data_storage(void** state)
{
    iolink_master_port_t port;
    const uint8_t initial_value[] = {0xAAU, 0x55U};
    const uint8_t updated_value[] = {0x10U, 0x20U, 0x30U};
    uint8_t i;

    (void)state;

    fake_iolink_device_set_data_storage(initial_value, sizeof(initial_value));

    assert_int_equal(iolink_master_init(&port, fake_iolink_device_phy(), &g_config), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);

    assert_int_equal(iolink_master_write_data_storage(&port, updated_value, sizeof(updated_value)),
                     IOLINK_MASTER_STATUS_PENDING);

    for(i = 0U; i < 24U; i++)
    {
        assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
        assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
    }

    assert_int_equal(iolink_master_write_data_storage(&port, updated_value, sizeof(updated_value)),
                     IOLINK_MASTER_STATUS_OK);

    assert_int_equal(iolink_master_verify_data_storage(&port, updated_value, sizeof(updated_value)),
                     IOLINK_MASTER_STATUS_PENDING);

    for(i = 0U; i < 24U; i++)
    {
        assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
        assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
    }

    assert_int_equal(iolink_master_verify_data_storage(&port, updated_value, sizeof(updated_value)),
                     IOLINK_MASTER_STATUS_OK);
}

static void test_fake_device_restores_data_storage_block(void** state)
{
    iolink_master_port_t port;
    const uint8_t initial_value[] = {0xAAU, 0x55U};
    const uint8_t restored_value[] = {0x21U, 0x43U, 0x65U};
    int ret;
    uint8_t i;

    (void)state;

    fake_iolink_device_set_data_storage(initial_value, sizeof(initial_value));

    assert_int_equal(iolink_master_init(&port, fake_iolink_device_phy(), &g_config), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);

    ret = iolink_master_restore_data_storage(&port, restored_value, sizeof(restored_value));
    assert_int_equal(ret, IOLINK_MASTER_STATUS_PENDING);

    for(i = 0U; i < 96U; i++)
    {
        assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
        (void)iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE);
        ret = iolink_master_restore_data_storage(&port, restored_value, sizeof(restored_value));
        if(ret == IOLINK_MASTER_STATUS_OK)
        {
            break;
        }
        assert_int_equal(ret, IOLINK_MASTER_STATUS_PENDING);
    }

    assert_int_equal(ret, IOLINK_MASTER_STATUS_OK);
    assert_int_equal(iolink_master_verify_data_storage(&port, restored_value, sizeof(restored_value)),
                     IOLINK_MASTER_STATUS_PENDING);

    for(i = 0U; i < 24U; i++)
    {
        assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
        assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
    }

    assert_int_equal(iolink_master_verify_data_storage(&port, restored_value, sizeof(restored_value)),
                     IOLINK_MASTER_STATUS_OK);
}

static void test_fake_device_serves_startup_device_validation_page(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;
    iolink_master_device_info_t info;
    uint8_t i;

    (void)state;

    config.validate_device_info = true;
    fake_iolink_device_set_direct_parameter_page1(10U,
                                                  0x03U,
                                                  0x08U,
                                                  0x00U,
                                                  0x1234U,
                                                  0x56789AU);

    assert_int_equal(iolink_master_init(&port, fake_iolink_device_phy(), &config), 0);

    for(i = 0U; i < 64U; i++)
    {
        assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
        (void)iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE);
        if(iolink_master_get_state(&port) == IOLINK_MASTER_STATE_OPERATE)
        {
            break;
        }
    }

    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);
    assert_int_equal(iolink_master_get_device_info(&port, &info), IOLINK_MASTER_STATUS_OK);
    assert_int_equal(info.vendor_id, 0x1234U);
    assert_int_equal(info.device_id, 0x56789AU);
}

static void test_fake_device_direct_parameter_profile_selects_compatible_config(void** state)
{
    iolink_master_port_t port;
    iolink_master_device_info_t info;
    iolink_master_config_t config = {
        .port_mode = IOLINK_MASTER_PORT_MODE_IOLINK,
        .baudrate = IOLINK_BAUDRATE_COM3,
        .auto_baudrate = true,
    };
    uint8_t i;

    (void)state;

    fake_iolink_device_set_direct_parameter_page1(12U,
                                                  0x0BU,
                                                  0x10U,
                                                  0x83U,
                                                  0x0102U,
                                                  0x030405U);

    assert_int_equal(iolink_master_init(&port, fake_iolink_device_phy(), &g_config), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_PREOPERATE);

    assert_int_equal(iolink_master_read_device_info(&port), IOLINK_MASTER_STATUS_PENDING);
    for(i = 0U; i < 39U; i++)
    {
        assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
        (void)iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE);
    }

    assert_int_equal(iolink_master_read_device_info(&port), IOLINK_MASTER_PARAM_ERR_PD_SIZE);
    assert_int_equal(iolink_master_get_device_info(&port, &info), IOLINK_MASTER_STATUS_OK);
    assert_int_equal(iolink_master_select_config_from_device_info(&info, &config),
                     IOLINK_MASTER_STATUS_OK);
    assert_int_equal(config.m_seq_type, IOLINK_MASTER_M_SEQ_TYPE_2_V);
    assert_int_equal(config.min_cycle_time, 12U);
    assert_int_equal(config.pd_in_len, 2U);
    assert_int_equal(config.pd_out_len, 4U);
    assert_int_equal(iolink_master_validate_config_against_device_info(&info, &config),
                     IOLINK_MASTER_STATUS_OK);
}

static void test_fake_device_keeps_startup_page_and_application_object(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;
    uint8_t data[8] = {0U};
    uint8_t len = sizeof(data);
    const uint8_t object_value[] = {0x4FU, 0x4BU};
    const uint8_t page1[] = {
        0x00U,
        0x00U,
        10U,
        0x03U,
        0x11U,
        0x08U,
        0x00U,
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
    uint8_t i;

    (void)state;

    config.validate_device_info = true;
    fake_iolink_device_set_isdu_object(IOLINK_IDX_DIRECT_PARAMETERS_1, 0U, page1, sizeof(page1));
    fake_iolink_device_set_isdu_object(0x0010U, 0U, object_value, sizeof(object_value));

    assert_int_equal(iolink_master_init(&port, fake_iolink_device_phy(), &config), 0);

    for(i = 0U; i < 64U; i++)
    {
        assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
        (void)iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE);
        if(iolink_master_get_state(&port) == IOLINK_MASTER_STATE_OPERATE)
        {
            break;
        }
    }

    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);
    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, data, &len),
                     IOLINK_MASTER_STATUS_PENDING);

    for(i = 0U; i < 11U; i++)
    {
        assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE), 0);
        assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_NONE), 1);
    }

    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, data, &len),
                     IOLINK_MASTER_STATUS_OK);
    assert_int_equal(len, 2U);
    assert_int_equal(data[0], 0x4FU);
    assert_int_equal(data[1], 0x4BU);
}

int main(void)
{
    const struct CMUnitTest tests[] = {
        cmocka_unit_test_setup(test_fake_device_drives_startup_and_paced_pd_cycle,
                               reset_fixture),
        cmocka_unit_test_setup(test_fake_device_conformance_matrix_nominal_profiles,
                               reset_fixture),
        cmocka_unit_test_setup(test_fake_device_can_inject_bad_operate_checksum,
                               reset_fixture),
        cmocka_unit_test_setup(test_fake_device_can_drop_response_for_timeout_path,
                               reset_fixture),
        cmocka_unit_test_setup(test_fake_device_truncated_response_is_discarded_after_timeout,
                               reset_fixture),
        cmocka_unit_test_setup(test_fake_device_exposes_event_pending_status,
                               reset_fixture),
        cmocka_unit_test_setup(test_fake_device_serves_event_details,
                               reset_fixture),
        cmocka_unit_test_setup(test_fake_device_ack_event_reads_event_code,
                               reset_fixture),
        cmocka_unit_test_setup(test_fake_device_dispatches_event_pending_on_rising_edge,
                               reset_fixture),
        cmocka_unit_test_setup(test_fake_device_dispatches_decoded_events_to_handler,
                               reset_fixture),
        cmocka_unit_test_setup(test_fake_device_serves_isdu_object_dictionary_read,
                               reset_fixture),
        cmocka_unit_test_setup(test_fake_device_accepts_isdu_object_dictionary_write,
                               reset_fixture),
        cmocka_unit_test_setup(test_fake_device_verifies_written_data_storage,
                               reset_fixture),
        cmocka_unit_test_setup(test_fake_device_restores_data_storage_block,
                               reset_fixture),
        cmocka_unit_test_setup(test_fake_device_serves_startup_device_validation_page,
                               reset_fixture),
        cmocka_unit_test_setup(test_fake_device_direct_parameter_profile_selects_compatible_config,
                               reset_fixture),
        cmocka_unit_test_setup(test_fake_device_keeps_startup_page_and_application_object,
                               reset_fixture),
    };

    return cmocka_run_group_tests(tests, NULL, NULL);
}
