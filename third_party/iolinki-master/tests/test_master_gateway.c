#include <setjmp.h>
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#include <cmocka.h>

#include "iolinki_master/master.h"
#include "../src/master_internal.h"

static int fake_send(void* user, const uint8_t* data, size_t len)
{
    (void)user;
    (void)data;
    return (int)len;
}

static int fake_recv_byte(void* user, uint8_t* byte)
{
    (void)user;
    (void)byte;
    return 0;
}

static const iolink_phy_api_t g_phy = {
    .send = fake_send,
    .recv_byte = fake_recv_byte,
};

static void test_format_gateway_line_encodes_identity_and_pd(void** state)
{
    iolink_master_device_info_t info;
    const uint8_t pd[] = {0xAAU, 0xBBU};
    char line[IOLINK_MASTER_GATEWAY_LINE_MAX];

    (void)state;
    memset(&info, 0, sizeof(info));
    info.valid = true;
    info.vendor_id = 0x0123U;
    info.device_id = 0x00045678U;

    assert_int_equal(iolink_master_format_gateway_line(0U, &info, pd, sizeof(pd), line, sizeof(line)),
                     IOLINK_MASTER_STATUS_OK);
    assert_string_equal(line, "iolinki-gw/1 0 0123 00045678 aabb\n");
}

static void test_format_gateway_line_uses_dash_for_empty_pd(void** state)
{
    iolink_master_device_info_t info;
    char line[IOLINK_MASTER_GATEWAY_LINE_MAX];

    (void)state;
    memset(&info, 0, sizeof(info));
    info.valid = true;
    info.vendor_id = 0xABCDU;
    info.device_id = 0x10U;

    assert_int_equal(iolink_master_format_gateway_line(2U, &info, NULL, 0U, line, sizeof(line)),
                     IOLINK_MASTER_STATUS_OK);
    assert_string_equal(line, "iolinki-gw/1 2 abcd 00000010 -\n");
}

static void test_format_gateway_line_rejects_invalid_info_and_short_buffer(void** state)
{
    iolink_master_device_info_t info;
    const uint8_t pd[] = {0x01U};
    char line[8];

    (void)state;
    memset(&info, 0, sizeof(info));

    assert_int_equal(iolink_master_format_gateway_line(0U, &info, pd, 1U, line, sizeof(line)),
                     IOLINK_MASTER_ERR_INVALID_ARG);
    info.valid = true;
    assert_int_equal(iolink_master_format_gateway_line(0U, &info, NULL, 1U, line, IOLINK_MASTER_GATEWAY_LINE_MAX),
                     IOLINK_MASTER_ERR_INVALID_ARG);
    assert_int_equal(iolink_master_format_gateway_line(0U, &info, pd, 33U, line, IOLINK_MASTER_GATEWAY_LINE_MAX),
                     IOLINK_MASTER_ERR_INVALID_ARG);
    info.vendor_id = 1U;
    memset(line, 'Z', sizeof(line));
    assert_int_equal(iolink_master_format_gateway_line(0U, &info, pd, 1U, line, sizeof(line)),
                     IOLINK_MASTER_ERR_BUFFER_TOO_SMALL);
    assert_int_equal(line[0], 'Z');
}

static void test_write_gateway_line_is_pending_before_a_device_is_known(void** state)
{
    iolink_master_port_t port;
    const iolink_master_config_t config = {
        .port_mode = IOLINK_MASTER_PORT_MODE_IOLINK,
        .m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_1_1,
        .baudrate = IOLINK_BAUDRATE_COM3,
        .pd_in_len = 1U,
    };
    char line[IOLINK_MASTER_GATEWAY_LINE_MAX];

    (void)state;
    assert_int_equal(iolink_master_init(&port, &g_phy, &config), IOLINK_MASTER_STATUS_OK);
    assert_int_equal(iolink_master_write_gateway_line(&port, 0U, line, sizeof(line)),
                     IOLINK_MASTER_STATUS_PENDING);
}

static void test_write_gateway_line_uses_dash_until_process_data_is_valid(void** state)
{
    iolink_master_port_t port;
    iolink_master_port_state_t* port_state;
    const iolink_master_config_t config = {
        .port_mode = IOLINK_MASTER_PORT_MODE_IOLINK,
        .m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_1_1,
        .baudrate = IOLINK_BAUDRATE_COM3,
        .pd_in_len = 1U,
    };
    char line[IOLINK_MASTER_GATEWAY_LINE_MAX];

    (void)state;
    assert_int_equal(iolink_master_init(&port, &g_phy, &config), IOLINK_MASTER_STATUS_OK);
    port_state = iolink_master_port_state(&port);
    port_state->device_info.valid = true;
    port_state->device_info.vendor_id = 0x0123U;
    port_state->device_info.device_id = 0x00045678U;
    port_state->pd_valid = false;

    assert_int_equal(iolink_master_write_gateway_line(&port, 0U, line, sizeof(line)),
                     IOLINK_MASTER_STATUS_OK);
    assert_string_equal(line, "iolinki-gw/1 0 0123 00045678 -\n");
}

static void test_write_gateway_line_emits_identity_and_process_data(void** state)
{
    iolink_master_port_t port;
    iolink_master_port_state_t* port_state;
    const iolink_master_config_t config = {
        .port_mode = IOLINK_MASTER_PORT_MODE_IOLINK,
        .m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_1_1,
        .baudrate = IOLINK_BAUDRATE_COM3,
        .pd_in_len = 2U,
    };
    char line[IOLINK_MASTER_GATEWAY_LINE_MAX];

    (void)state;
    assert_int_equal(iolink_master_init(&port, &g_phy, &config), IOLINK_MASTER_STATUS_OK);
    port_state = iolink_master_port_state(&port);
    port_state->device_info.valid = true;
    port_state->device_info.vendor_id = 0x0123U;
    port_state->device_info.device_id = 0x00045678U;
    port_state->pd_in[0] = 0xAAU;
    port_state->pd_in[1] = 0xBBU;
    port_state->pd_in_len = 2U;
    port_state->pd_valid = true;

    assert_int_equal(iolink_master_write_gateway_line(&port, 0U, line, sizeof(line)),
                     IOLINK_MASTER_STATUS_OK);
    assert_string_equal(line, "iolinki-gw/1 0 0123 00045678 aabb\n");
}

int main(void)
{
    const struct CMUnitTest tests[] = {
        cmocka_unit_test(test_format_gateway_line_encodes_identity_and_pd),
        cmocka_unit_test(test_format_gateway_line_uses_dash_for_empty_pd),
        cmocka_unit_test(test_format_gateway_line_rejects_invalid_info_and_short_buffer),
        cmocka_unit_test(test_write_gateway_line_is_pending_before_a_device_is_known),
        cmocka_unit_test(test_write_gateway_line_uses_dash_until_process_data_is_valid),
        cmocka_unit_test(test_write_gateway_line_emits_identity_and_process_data),
    };

    return cmocka_run_group_tests(tests, NULL, NULL);
}
