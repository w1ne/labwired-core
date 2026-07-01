#include <setjmp.h>
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#include <cmocka.h>

#include "iolinki/crc.h"
#include "iolinki/protocol.h"
#include "../src/master_internal.h"

static const iolink_phy_api_t g_empty_phy = {0};

static uint8_t g_recv_bytes[16];
static uint8_t g_recv_len;
static uint8_t g_recv_pos;
static int g_recv_error_after = -1;

static int fake_recv_byte(void* user, uint8_t* byte)
{
    (void)user;
    assert_non_null(byte);

    if((g_recv_error_after >= 0) && (g_recv_pos >= (uint8_t)g_recv_error_after))
    {
        return -1;
    }

    if(g_recv_pos >= g_recv_len)
    {
        return 0;
    }

    *byte = g_recv_bytes[g_recv_pos++];
    return 1;
}

static void load_recv_bytes(const uint8_t* data, uint8_t len)
{
    assert_in_range(len, 0, sizeof(g_recv_bytes));
    memcpy(g_recv_bytes, data, len);
    g_recv_len = len;
    g_recv_pos = 0U;
    g_recv_error_after = -1;
}

static const iolink_phy_api_t g_recv_phy = {
    .recv_byte = fake_recv_byte,
};

static const iolink_master_config_t g_config = {
    .m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_2_1,
    .baudrate = IOLINK_BAUDRATE_COM3,
    .min_cycle_time = 20U,
    .pd_in_len = 4U,
    .pd_out_len = 2U,
};

static void test_on_rx_valid_response_latches_pd(void** state)
{
    iolink_master_port_t port = {0};
    const uint8_t frame[] = {0x20U, 0xA5U, 0x00U, 0x0DU};
    uint8_t pd[1] = {0U};
    uint8_t len = 0U;

    (void)state;

    iolink_master_port_state(&port)->config.pd_in_len = 1U;
    iolink_master_port_state(&port)->od_len = 1U;

    assert_int_equal(iolink_master_on_rx(&port, frame, sizeof(frame)), 0);
    assert_int_equal(iolink_master_get_pd_in(&port, pd, sizeof(pd), &len), 0);
    assert_int_equal(len, 1U);
    assert_int_equal(pd[0], 0xA5U);
}

static void test_poll_rx_latches_complete_operate_response_from_phy(void** state)
{
    iolink_master_port_t port = {0};
    const uint8_t frame[] = {0x20U, 0xA5U, 0x00U, 0x0DU};
    uint8_t pd[1] = {0U};
    uint8_t len = 0U;

    (void)state;

    iolink_master_port_state(&port)->phy = &g_recv_phy;
    iolink_master_port_state(&port)->state = IOLINK_MASTER_STATE_OPERATE;
    iolink_master_port_state(&port)->config.pd_in_len = 1U;
    iolink_master_port_state(&port)->od_len = 1U;
    load_recv_bytes(frame, sizeof(frame));

    assert_int_equal(iolink_master_poll_rx(&port), 1);
    assert_int_equal(iolink_master_get_pd_in(&port, pd, sizeof(pd), &len), 0);
    assert_int_equal(len, 1U);
    assert_int_equal(pd[0], 0xA5U);
}

static void test_poll_rx_keeps_partial_response_until_complete(void** state)
{
    iolink_master_port_t port = {0};
    const uint8_t partial[] = {0x20U, 0xA5U};
    const uint8_t rest[] = {0x00U, 0x0DU};
    uint8_t pd[1] = {0U};
    uint8_t len = 0U;

    (void)state;

    iolink_master_port_state(&port)->phy = &g_recv_phy;
    iolink_master_port_state(&port)->state = IOLINK_MASTER_STATE_OPERATE;
    iolink_master_port_state(&port)->config.pd_in_len = 1U;
    iolink_master_port_state(&port)->od_len = 1U;

    load_recv_bytes(partial, sizeof(partial));
    assert_int_equal(iolink_master_poll_rx(&port), 0);
    assert_int_equal(iolink_master_get_pd_in(&port, pd, sizeof(pd), &len), 1);

    load_recv_bytes(rest, sizeof(rest));
    assert_int_equal(iolink_master_poll_rx(&port), 1);
    assert_int_equal(iolink_master_get_pd_in(&port, pd, sizeof(pd), &len), 0);
    assert_int_equal(pd[0], 0xA5U);
}

static void test_poll_rx_enters_error_on_phy_receive_error(void** state)
{
    iolink_master_port_t port = {0};
    const uint8_t frame[] = {0x20U};

    (void)state;

    iolink_master_port_state(&port)->phy = &g_recv_phy;
    iolink_master_port_state(&port)->state = IOLINK_MASTER_STATE_OPERATE;
    iolink_master_port_state(&port)->config.pd_in_len = 1U;
    iolink_master_port_state(&port)->od_len = 1U;
    load_recv_bytes(frame, sizeof(frame));
    g_recv_error_after = 1;

    assert_int_equal(iolink_master_poll_rx(&port), -2);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_ERROR);
}

static void test_on_rx_latches_od_status_for_diagnostics(void** state)
{
    iolink_master_port_t port = {0};
    uint8_t frame[] = {0xA3U, 0xA5U, 0x00U, 0x00U};
    uint8_t status = 0U;
    iolink_master_diagnostics_t diagnostics;

    (void)state;

    iolink_master_port_state(&port)->config.pd_in_len = 1U;
    iolink_master_port_state(&port)->od_len = 1U;
    frame[3] = iolink_crc6(frame, 3U);

    assert_int_equal(iolink_master_on_rx(&port, frame, sizeof(frame)), 0);
    assert_int_equal(iolink_master_get_od_status(&port, &status), 0);
    assert_int_equal(status, 0xA3U);
    assert_true(iolink_master_port_state(&port)->diagnostics.event_pending);
    assert_int_equal(iolink_master_get_device_status(&port), IOLINK_DEVICE_STATUS_FAILURE);
    assert_int_equal(iolink_master_get_diagnostics(&port, &diagnostics), 0);
    assert_int_equal(diagnostics.od_status, 0xA3U);
    assert_true(diagnostics.event_pending);
    assert_int_equal(diagnostics.link_quality_percent, 100U);
}

static void test_diagnostics_reports_derived_link_quality(void** state)
{
    iolink_master_port_t port;
    iolink_master_diagnostics_t diagnostics;

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_empty_phy, &g_config), 0);
    iolink_master_port_state(&port)->cycle_count = 7U;
    iolink_master_port_state(&port)->diagnostics.checksum_errors = 1U;
    iolink_master_port_state(&port)->diagnostics.response_timeouts = 1U;
    iolink_master_port_state(&port)->diagnostics.send_errors = 1U;

    assert_int_equal(iolink_master_get_diagnostics(&port, &diagnostics), 0);
    assert_int_equal(diagnostics.link_quality_percent, 70U);
}

static void test_get_od_status_rejects_invalid_args(void** state)
{
    iolink_master_port_t port = {0};
    uint8_t status = 0U;
    iolink_master_diagnostics_t diagnostics;

    (void)state;

    assert_int_equal(iolink_master_get_od_status(NULL, &status), -1);
    assert_int_equal(iolink_master_get_od_status(&port, NULL), -1);
    assert_int_equal(iolink_master_get_diagnostics(NULL, &diagnostics), -1);
    assert_int_equal(iolink_master_get_diagnostics(&port, NULL), -1);
}

static void test_get_device_status_returns_failure_for_null_port(void** state)
{
    (void)state;

    assert_int_equal(iolink_master_get_device_status(NULL), IOLINK_DEVICE_STATUS_FAILURE);
}

static void test_on_rx_bad_checksum_returns_error_and_increments_count(void** state)
{
    iolink_master_port_t port = {0};
    const uint8_t frame[] = {0x20U, 0xA5U, 0x00U, 0x00U};

    (void)state;

    iolink_master_port_state(&port)->config.pd_in_len = 1U;
    iolink_master_port_state(&port)->od_len = 1U;

    assert_int_equal(iolink_master_on_rx(&port, frame, sizeof(frame)), -3);
    assert_int_equal(iolink_master_port_state(&port)->diagnostics.checksum_errors, 1U);
}

static void test_on_rx_bad_checksum_retries_twice_before_error_state(void** state)
{
    iolink_master_port_t port = {0};
    const uint8_t frame[] = {0x20U, 0xA5U, 0x00U, 0x00U};

    (void)state;

    iolink_master_port_state(&port)->state = IOLINK_MASTER_STATE_OPERATE;
    iolink_master_port_state(&port)->config.pd_in_len = 1U;
    iolink_master_port_state(&port)->od_len = 1U;

    assert_int_equal(iolink_master_on_rx(&port, frame, sizeof(frame)), -3);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);

    assert_int_equal(iolink_master_on_rx(&port, frame, sizeof(frame)), -3);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);

    assert_int_equal(iolink_master_on_rx(&port, frame, sizeof(frame)), -3);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_ERROR);
    assert_int_equal(iolink_master_port_state(&port)->diagnostics.checksum_errors, 3U);
}

static void test_on_rx_valid_response_resets_checksum_retry_count(void** state)
{
    iolink_master_port_t port = {0};
    const uint8_t bad_frame[] = {0x20U, 0xA5U, 0x00U, 0x00U};
    const uint8_t good_frame[] = {0x20U, 0xA5U, 0x00U, 0x0DU};

    (void)state;

    iolink_master_port_state(&port)->state = IOLINK_MASTER_STATE_OPERATE;
    iolink_master_port_state(&port)->config.pd_in_len = 1U;
    iolink_master_port_state(&port)->od_len = 1U;

    assert_int_equal(iolink_master_on_rx(&port, bad_frame, sizeof(bad_frame)), -3);
    assert_int_equal(iolink_master_on_rx(&port, good_frame, sizeof(good_frame)), 0);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);

    assert_int_equal(iolink_master_on_rx(&port, bad_frame, sizeof(bad_frame)), -3);
    assert_int_equal(iolink_master_on_rx(&port, bad_frame, sizeof(bad_frame)), -3);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);
}

static void test_operate_timeout_retries_twice_before_error_state(void** state)
{
    iolink_master_port_t port = {0};

    (void)state;

    iolink_master_port_state(&port)->state = IOLINK_MASTER_STATE_OPERATE;

    assert_int_equal(iolink_master_on_timeout(&port), 1);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);

    assert_int_equal(iolink_master_on_timeout(&port), 1);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);

    assert_int_equal(iolink_master_on_timeout(&port), -2);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_ERROR);
}

static void test_valid_response_resets_operate_timeout_retry_count(void** state)
{
    iolink_master_port_t port = {0};
    const uint8_t good_frame[] = {0x20U, 0xA5U, 0x00U, 0x0DU};

    (void)state;

    iolink_master_port_state(&port)->state = IOLINK_MASTER_STATE_OPERATE;
    iolink_master_port_state(&port)->config.pd_in_len = 1U;
    iolink_master_port_state(&port)->od_len = 1U;

    assert_int_equal(iolink_master_on_timeout(&port), 1);
    assert_int_equal(iolink_master_on_rx(&port, good_frame, sizeof(good_frame)), 0);

    assert_int_equal(iolink_master_on_timeout(&port), 1);
    assert_int_equal(iolink_master_on_timeout(&port), 1);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_OPERATE);
}

static void test_on_rx_malformed_frame_returns_decode_error(void** state)
{
    iolink_master_port_t port = {0};
    const uint8_t frame[] = {0x20U, 0xA5U};

    (void)state;

    iolink_master_port_state(&port)->config.pd_in_len = 1U;
    iolink_master_port_state(&port)->od_len = 1U;

    assert_int_equal(iolink_master_on_rx(&port, frame, sizeof(frame)), -2);
    assert_int_equal(iolink_master_port_state(&port)->diagnostics.checksum_errors, 0U);
}

static void test_on_rx_rejects_invalid_args(void** state)
{
    iolink_master_port_t port = {0};
    const uint8_t frame[] = {0x20U, 0xA5U, 0x00U, 0x0DU};

    (void)state;

    assert_int_equal(iolink_master_on_rx(NULL, frame, sizeof(frame)), -1);
    assert_int_equal(iolink_master_on_rx(&port, NULL, sizeof(frame)), -1);
    assert_int_equal(iolink_master_on_rx(&port, frame, 0U), -1);
}

static void test_set_pd_out_rejects_invalid_args(void** state)
{
    iolink_master_port_t port;
    const uint8_t pd_out[] = {0x11U, 0x22U};

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_empty_phy, &g_config), 0);
    assert_int_equal(iolink_master_set_pd_out(NULL, pd_out, sizeof(pd_out)), -1);
    assert_int_equal(iolink_master_set_pd_out(&port, NULL, sizeof(pd_out)), -1);
}

static void test_set_pd_out_rejects_length_mismatch(void** state)
{
    iolink_master_port_t port;
    const uint8_t short_pd_out[] = {0x11U};
    const uint8_t long_pd_out[] = {0x11U, 0x22U, 0x33U};

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_empty_phy, &g_config), 0);
    assert_int_equal(iolink_master_set_pd_out(&port, short_pd_out, sizeof(short_pd_out)), -2);
    assert_int_equal(iolink_master_set_pd_out(&port, long_pd_out, sizeof(long_pd_out)), -2);
}

static void test_set_pd_out_accepts_zero_length_when_configured(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = g_config;

    (void)state;

    config.pd_out_len = 0U;

    assert_int_equal(iolink_master_init(&port, &g_empty_phy, &config), 0);
    assert_int_equal(iolink_master_set_pd_out(&port, NULL, 0U), 0);
    assert_int_equal(iolink_master_port_state(&port)->pd_out_len, 0U);
}

int main(void)
{
    const struct CMUnitTest tests[] = {
        cmocka_unit_test(test_on_rx_valid_response_latches_pd),
        cmocka_unit_test(test_poll_rx_latches_complete_operate_response_from_phy),
        cmocka_unit_test(test_poll_rx_keeps_partial_response_until_complete),
        cmocka_unit_test(test_poll_rx_enters_error_on_phy_receive_error),
        cmocka_unit_test(test_on_rx_latches_od_status_for_diagnostics),
        cmocka_unit_test(test_diagnostics_reports_derived_link_quality),
        cmocka_unit_test(test_get_od_status_rejects_invalid_args),
        cmocka_unit_test(test_get_device_status_returns_failure_for_null_port),
        cmocka_unit_test(test_on_rx_bad_checksum_returns_error_and_increments_count),
        cmocka_unit_test(test_on_rx_bad_checksum_retries_twice_before_error_state),
        cmocka_unit_test(test_on_rx_valid_response_resets_checksum_retry_count),
        cmocka_unit_test(test_operate_timeout_retries_twice_before_error_state),
        cmocka_unit_test(test_valid_response_resets_operate_timeout_retry_count),
        cmocka_unit_test(test_on_rx_malformed_frame_returns_decode_error),
        cmocka_unit_test(test_on_rx_rejects_invalid_args),
        cmocka_unit_test(test_set_pd_out_rejects_invalid_args),
        cmocka_unit_test(test_set_pd_out_rejects_length_mismatch),
        cmocka_unit_test(test_set_pd_out_accepts_zero_length_when_configured),
    };

    return cmocka_run_group_tests(tests, NULL, NULL);
}
