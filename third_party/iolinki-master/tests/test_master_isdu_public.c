#include <setjmp.h>
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#include <cmocka.h>

#include "iolinki/crc.h"
#include "iolinki/protocol.h"
#include "iolinki_master/master.h"

static int g_send_calls;
static uint8_t g_sent[48][8];
static size_t g_sent_len[48];

static int fake_send(void* user, const uint8_t* data, size_t len)
{
    (void)user;
    assert_non_null(data);
    assert_in_range(g_send_calls, 0, 47);
    assert_in_range(len, 1U, sizeof(g_sent[0]));

    memcpy(g_sent[g_send_calls], data, len);
    g_sent_len[g_send_calls] = len;
    g_send_calls++;
    return (int)len;
}

static const iolink_phy_api_t g_phy = {
    .send = fake_send,
};

static const iolink_master_config_t g_config = {
    .port_mode = IOLINK_MASTER_PORT_MODE_IOLINK,
    .m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_0,
    .baudrate = IOLINK_BAUDRATE_COM3,
    .min_cycle_time = 20U,
};

static int reset_fixture(void** state)
{
    (void)state;
    g_send_calls = 0;
    memset(g_sent, 0, sizeof(g_sent));
    memset(g_sent_len, 0, sizeof(g_sent_len));
    return 0;
}

static void feed_type0_byte(iolink_master_port_t* port, uint8_t byte)
{
    uint8_t frame[2];

    frame[0] = byte;
    frame[1] = iolink_checksum_ck(frame[0], 0U);
    assert_int_equal(iolink_master_on_rx(port, frame, sizeof(frame)), IOLINK_MASTER_STATUS_OK);
}

static void enter_type0_operate(iolink_master_port_t* port)
{
    assert_int_equal(iolink_master_init(port, &g_phy, &g_config), IOLINK_MASTER_STATUS_OK);
    assert_int_equal(iolink_master_tick_event(port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_int_equal(g_sent_len[0], 1U);
    assert_int_equal(g_sent[0][0], 0x55U);

    assert_int_equal(iolink_master_tick_event(port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    feed_type0_byte(port, 0x00U);
    assert_int_equal(iolink_master_get_state(port), IOLINK_MASTER_STATE_PREOPERATE);

    assert_int_equal(iolink_master_tick_event(port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_int_equal(iolink_master_get_state(port), IOLINK_MASTER_STATE_OPERATE);
}

static void assert_last_type0_request(uint8_t expected_od)
{
    assert_true(g_send_calls > 0);
    assert_int_equal(g_sent_len[g_send_calls - 1], IOLINK_M_SEQ_TYPE0_LEN);
    assert_int_equal(g_sent[g_send_calls - 1][0], expected_od);
    assert_int_equal(g_sent[g_send_calls - 1][1], iolink_checksum_ck(expected_od, 0U));
}

static void assert_next_type0_request(iolink_master_port_t* port, uint8_t expected_od)
{
    assert_int_equal(iolink_master_tick_event(port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_last_type0_request(expected_od);
}

static void test_public_type0_isdu_read_completes_without_private_state(void** state)
{
    iolink_master_port_t port;
    uint8_t data[4] = {0U};
    uint8_t len = sizeof(data);

    (void)state;

    enter_type0_operate(&port);

    assert_int_equal(iolink_master_read_isdu(&port, 0x1234U, 0x56U, data, &len),
                     IOLINK_MASTER_STATUS_PENDING);

    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_last_type0_request(IOLINK_ISDU_CTRL_START);

    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_last_type0_request(IOLINK_ISDU_SERVICE_READ << 4);

    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_last_type0_request(0x01U);

    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_last_type0_request(0x12U);

    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_last_type0_request(0x02U);

    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_last_type0_request(0x34U);

    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_last_type0_request((uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x03U));

    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_last_type0_request(0x56U);

    feed_type0_byte(&port, IOLINK_ISDU_CTRL_START);
    feed_type0_byte(&port, 0xCAU);
    feed_type0_byte(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x01U));
    feed_type0_byte(&port, 0xFEU);

    assert_int_equal(iolink_master_read_isdu(&port, 0x1234U, 0x56U, data, &len),
                     IOLINK_MASTER_STATUS_OK);
    assert_int_equal(len, 2U);
    assert_int_equal(data[0], 0xCAU);
    assert_int_equal(data[1], 0xFEU);
}

static void test_public_data_storage_read_uses_standard_index(void** state)
{
    iolink_master_port_t port;
    uint8_t data[8] = {0U};
    uint8_t len = sizeof(data);

    (void)state;

    enter_type0_operate(&port);

    assert_int_equal(iolink_master_read_data_storage(&port, data, &len),
                     IOLINK_MASTER_STATUS_PENDING);

    assert_next_type0_request(&port, IOLINK_ISDU_CTRL_START);
    assert_next_type0_request(&port, IOLINK_ISDU_SERVICE_READ << 4);
    assert_next_type0_request(&port, 0x01U);
    assert_next_type0_request(&port, 0x00U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, 0x03U);
    assert_next_type0_request(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x03U));
    assert_next_type0_request(&port, 0x00U);
}

static void test_public_detailed_device_status_read_uses_standard_index(void** state)
{
    iolink_master_port_t port;
    uint8_t data[8] = {0U};
    uint8_t len = sizeof(data);

    (void)state;

    enter_type0_operate(&port);

    assert_int_equal(iolink_master_read_detailed_device_status(&port, data, &len),
                     IOLINK_MASTER_STATUS_PENDING);

    assert_next_type0_request(&port, IOLINK_ISDU_CTRL_START);
    assert_next_type0_request(&port, IOLINK_ISDU_SERVICE_READ << 4);
    assert_next_type0_request(&port, 0x01U);
    assert_next_type0_request(&port, 0x00U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, 0x1CU);
    assert_next_type0_request(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x03U));
    assert_next_type0_request(&port, 0x00U);
}

static void test_public_event_code_read_uses_standard_index_and_decodes(void** state)
{
    iolink_master_port_t port;
    uint16_t event_code = 0U;

    (void)state;

    enter_type0_operate(&port);

    assert_int_equal(iolink_master_read_event_code(&port, &event_code),
                     IOLINK_MASTER_STATUS_PENDING);

    assert_next_type0_request(&port, IOLINK_ISDU_CTRL_START);
    assert_next_type0_request(&port, IOLINK_ISDU_SERVICE_READ << 4);
    assert_next_type0_request(&port, 0x01U);
    assert_next_type0_request(&port, 0x00U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x03U));
    assert_next_type0_request(&port, 0x00U);

    feed_type0_byte(&port, IOLINK_ISDU_CTRL_START);
    feed_type0_byte(&port, 0x18U);
    feed_type0_byte(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x01U));
    feed_type0_byte(&port, 0x03U);

    assert_int_equal(iolink_master_read_event_code(&port, &event_code),
                     IOLINK_MASTER_STATUS_OK);
    assert_int_equal(event_code, 0x1803U);
}

static void test_public_event_ack_reads_and_returns_event_code(void** state)
{
    iolink_master_port_t port;
    uint16_t event_code = 0U;

    (void)state;

    enter_type0_operate(&port);

    assert_int_equal(iolink_master_ack_event(&port, &event_code),
                     IOLINK_MASTER_STATUS_PENDING);

    assert_next_type0_request(&port, IOLINK_ISDU_CTRL_START);
    assert_next_type0_request(&port, IOLINK_ISDU_SERVICE_READ << 4);
    assert_next_type0_request(&port, 0x01U);
    assert_next_type0_request(&port, 0x00U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x03U));
    assert_next_type0_request(&port, 0x00U);

    feed_type0_byte(&port, IOLINK_ISDU_CTRL_START);
    feed_type0_byte(&port, 0x18U);
    feed_type0_byte(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x01U));
    feed_type0_byte(&port, 0x03U);

    assert_int_equal(iolink_master_ack_event(&port, &event_code), IOLINK_MASTER_STATUS_OK);
    assert_int_equal(event_code, 0x1803U);
}

static void test_public_event_details_read_decodes_detailed_device_status(void** state)
{
    iolink_master_port_t port;
    iolink_master_event_t events[2];
    uint8_t count = 0U;

    (void)state;

    memset(events, 0, sizeof(events));
    enter_type0_operate(&port);

    assert_int_equal(iolink_master_read_event_details(&port,
                                                      events,
                                                      (uint8_t)(sizeof(events) / sizeof(events[0])),
                                                      &count),
                     IOLINK_MASTER_STATUS_PENDING);

    assert_next_type0_request(&port, IOLINK_ISDU_CTRL_START);
    assert_next_type0_request(&port, IOLINK_ISDU_SERVICE_READ << 4);
    assert_next_type0_request(&port, 0x01U);
    assert_next_type0_request(&port, 0x00U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, 0x1CU);
    assert_next_type0_request(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x03U));
    assert_next_type0_request(&port, 0x00U);

    feed_type0_byte(&port, IOLINK_ISDU_CTRL_START);
    feed_type0_byte(&port, 0xE2U);
    feed_type0_byte(&port, 0x01U);
    feed_type0_byte(&port, 0x42U);
    feed_type0_byte(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x02U));
    feed_type0_byte(&port, 0x10U);

    assert_int_equal(iolink_master_read_event_details(&port,
                                                      events,
                                                      (uint8_t)(sizeof(events) / sizeof(events[0])),
                                                      &count),
                     IOLINK_MASTER_STATUS_OK);
    assert_int_equal(count, 1U);
    assert_int_equal(events[0].qualifier, 0xE2U);
    assert_int_equal(events[0].type, IOLINK_MASTER_EVENT_TYPE_WARNING);
    assert_int_equal(events[0].code, 0x4210U);
}

static void test_public_isdu_verify_readback_compares_value(void** state)
{
    iolink_master_port_t port;
    const uint8_t expected[] = {0x12U, 0x34U};
    const uint8_t mismatch[] = {0x12U, 0x35U};

    (void)state;

    enter_type0_operate(&port);

    assert_int_equal(iolink_master_verify_isdu(&port, 0x0010U, 0U, expected, sizeof(expected)),
                     IOLINK_MASTER_STATUS_PENDING);

    assert_next_type0_request(&port, IOLINK_ISDU_CTRL_START);
    assert_next_type0_request(&port, IOLINK_ISDU_SERVICE_READ << 4);
    assert_next_type0_request(&port, 0x01U);
    assert_next_type0_request(&port, 0x00U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, 0x10U);
    assert_next_type0_request(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x03U));
    assert_next_type0_request(&port, 0x00U);

    feed_type0_byte(&port, IOLINK_ISDU_CTRL_START);
    feed_type0_byte(&port, 0x12U);
    feed_type0_byte(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x01U));
    feed_type0_byte(&port, 0x34U);

    assert_int_equal(iolink_master_verify_isdu(&port, 0x0010U, 0U, expected, sizeof(expected)),
                     IOLINK_MASTER_STATUS_OK);

    assert_int_equal(iolink_master_verify_isdu(&port, 0x0010U, 0U, mismatch, sizeof(mismatch)),
                     IOLINK_MASTER_STATUS_PENDING);

    feed_type0_byte(&port, IOLINK_ISDU_CTRL_START);
    feed_type0_byte(&port, 0x12U);
    feed_type0_byte(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x01U));
    feed_type0_byte(&port, 0x34U);

    assert_int_equal(iolink_master_verify_isdu(&port, 0x0010U, 0U, mismatch, sizeof(mismatch)),
                     IOLINK_MASTER_ISDU_ERR_VERIFY_FAILED);
}

static void test_public_data_storage_verify_uses_standard_index(void** state)
{
    iolink_master_port_t port;
    const uint8_t expected[] = {0xDEU, 0xADU};

    (void)state;

    enter_type0_operate(&port);

    assert_int_equal(iolink_master_verify_data_storage(&port, expected, sizeof(expected)),
                     IOLINK_MASTER_STATUS_PENDING);

    assert_next_type0_request(&port, IOLINK_ISDU_CTRL_START);
    assert_next_type0_request(&port, IOLINK_ISDU_SERVICE_READ << 4);
    assert_next_type0_request(&port, 0x01U);
    assert_next_type0_request(&port, 0x00U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, 0x03U);
    assert_next_type0_request(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x03U));
    assert_next_type0_request(&port, 0x00U);

    feed_type0_byte(&port, IOLINK_ISDU_CTRL_START);
    feed_type0_byte(&port, 0xDEU);
    feed_type0_byte(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x01U));
    feed_type0_byte(&port, 0xADU);

    assert_int_equal(iolink_master_verify_data_storage(&port, expected, sizeof(expected)),
                     IOLINK_MASTER_STATUS_OK);
}

static void test_public_parameter_download_helpers_write_system_commands(void** state)
{
    iolink_master_port_t port;

    (void)state;

    enter_type0_operate(&port);

    assert_int_equal(iolink_master_begin_parameter_download(&port),
                     IOLINK_MASTER_STATUS_PENDING);
    assert_next_type0_request(&port, IOLINK_ISDU_CTRL_START);
    assert_next_type0_request(&port, (uint8_t)((IOLINK_ISDU_SERVICE_WRITE << 4) | 1U));
    assert_next_type0_request(&port, 0x01U);
    assert_next_type0_request(&port, 0x00U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, 0x03U);
    assert_next_type0_request(&port, 0x00U);
    assert_next_type0_request(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x04U));
    assert_next_type0_request(&port, IOLINK_CMD_PARAM_DOWNLOAD_START);

    feed_type0_byte(&port, (uint8_t)(IOLINK_ISDU_CTRL_START | IOLINK_ISDU_CTRL_LAST));
    feed_type0_byte(&port, 0x00U);
    assert_int_equal(iolink_master_begin_parameter_download(&port), IOLINK_MASTER_STATUS_OK);

    assert_int_equal(iolink_master_end_parameter_download(&port), IOLINK_MASTER_STATUS_PENDING);
    assert_next_type0_request(&port, IOLINK_ISDU_CTRL_START);
    assert_next_type0_request(&port, (uint8_t)((IOLINK_ISDU_SERVICE_WRITE << 4) | 1U));
    assert_next_type0_request(&port, 0x01U);
    assert_next_type0_request(&port, 0x00U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, 0x03U);
    assert_next_type0_request(&port, 0x00U);
    assert_next_type0_request(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x04U));
    assert_next_type0_request(&port, IOLINK_CMD_PARAM_DOWNLOAD_END);
}

static void test_public_parameter_upload_and_store_helpers_write_system_commands(void** state)
{
    iolink_master_port_t port;

    (void)state;

    enter_type0_operate(&port);

    assert_int_equal(iolink_master_begin_parameter_upload(&port),
                     IOLINK_MASTER_STATUS_PENDING);
    assert_next_type0_request(&port, IOLINK_ISDU_CTRL_START);
    assert_next_type0_request(&port, (uint8_t)((IOLINK_ISDU_SERVICE_WRITE << 4) | 1U));
    assert_next_type0_request(&port, 0x01U);
    assert_next_type0_request(&port, 0x00U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, 0x03U);
    assert_next_type0_request(&port, 0x00U);
    assert_next_type0_request(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x04U));
    assert_next_type0_request(&port, IOLINK_CMD_PARAM_UPLOAD_START);

    feed_type0_byte(&port, (uint8_t)(IOLINK_ISDU_CTRL_START | IOLINK_ISDU_CTRL_LAST));
    feed_type0_byte(&port, 0x00U);
    assert_int_equal(iolink_master_begin_parameter_upload(&port), IOLINK_MASTER_STATUS_OK);

    assert_int_equal(iolink_master_end_parameter_upload(&port), IOLINK_MASTER_STATUS_PENDING);
    assert_next_type0_request(&port, IOLINK_ISDU_CTRL_START);
    assert_next_type0_request(&port, (uint8_t)((IOLINK_ISDU_SERVICE_WRITE << 4) | 1U));
    assert_next_type0_request(&port, 0x01U);
    assert_next_type0_request(&port, 0x00U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, 0x03U);
    assert_next_type0_request(&port, 0x00U);
    assert_next_type0_request(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x04U));
    assert_next_type0_request(&port, IOLINK_CMD_PARAM_UPLOAD_END);

    feed_type0_byte(&port, (uint8_t)(IOLINK_ISDU_CTRL_START | IOLINK_ISDU_CTRL_LAST));
    feed_type0_byte(&port, 0x00U);
    assert_int_equal(iolink_master_end_parameter_upload(&port), IOLINK_MASTER_STATUS_OK);

    assert_int_equal(iolink_master_store_parameter_download(&port),
                     IOLINK_MASTER_STATUS_PENDING);
    assert_next_type0_request(&port, IOLINK_ISDU_CTRL_START);
    assert_next_type0_request(&port, (uint8_t)((IOLINK_ISDU_SERVICE_WRITE << 4) | 1U));
    assert_next_type0_request(&port, 0x01U);
    assert_next_type0_request(&port, 0x00U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, 0x03U);
    assert_next_type0_request(&port, 0x00U);
    assert_next_type0_request(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x04U));
    assert_next_type0_request(&port, IOLINK_CMD_PARAM_DOWNLOAD_STORE);
}

static void test_public_parameter_block_write_sequences_commands_and_readback(void** state)
{
    iolink_master_port_t port;
    iolink_master_diagnostics_t diagnostics;
    const uint8_t value[] = {0x12U, 0x34U};

    (void)state;

    enter_type0_operate(&port);

    assert_int_equal(iolink_master_write_parameter_block(&port,
                                                        0x0040U,
                                                        0x01U,
                                                        value,
                                                        sizeof(value)),
                     IOLINK_MASTER_STATUS_PENDING);
    assert_next_type0_request(&port, IOLINK_ISDU_CTRL_START);
    assert_next_type0_request(&port, (uint8_t)((IOLINK_ISDU_SERVICE_WRITE << 4) | 1U));
    assert_next_type0_request(&port, 0x01U);
    assert_next_type0_request(&port, 0x00U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, 0x03U);
    assert_next_type0_request(&port, 0x00U);
    assert_next_type0_request(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x04U));
    assert_next_type0_request(&port, IOLINK_CMD_PARAM_DOWNLOAD_START);

    feed_type0_byte(&port, (uint8_t)(IOLINK_ISDU_CTRL_START | IOLINK_ISDU_CTRL_LAST));
    feed_type0_byte(&port, 0x00U);
    assert_int_equal(iolink_master_write_parameter_block(&port,
                                                        0x0040U,
                                                        0x01U,
                                                        value,
                                                        sizeof(value)),
                     IOLINK_MASTER_STATUS_PENDING);

    assert_next_type0_request(&port, IOLINK_ISDU_CTRL_START);
    assert_next_type0_request(&port, (uint8_t)((IOLINK_ISDU_SERVICE_WRITE << 4) | 2U));
    assert_next_type0_request(&port, 0x01U);
    assert_next_type0_request(&port, 0x00U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, 0x40U);
    assert_next_type0_request(&port, 0x03U);
    assert_next_type0_request(&port, 0x01U);
    assert_next_type0_request(&port, 0x04U);
    assert_next_type0_request(&port, 0x12U);
    assert_next_type0_request(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x05U));
    assert_next_type0_request(&port, 0x34U);

    feed_type0_byte(&port, (uint8_t)(IOLINK_ISDU_CTRL_START | IOLINK_ISDU_CTRL_LAST));
    feed_type0_byte(&port, 0x00U);
    assert_int_equal(iolink_master_write_parameter_block(&port,
                                                        0x0040U,
                                                        0x01U,
                                                        value,
                                                        sizeof(value)),
                     IOLINK_MASTER_STATUS_PENDING);

    assert_next_type0_request(&port, IOLINK_ISDU_CTRL_START);
    assert_next_type0_request(&port, (uint8_t)((IOLINK_ISDU_SERVICE_WRITE << 4) | 1U));
    assert_next_type0_request(&port, 0x01U);
    assert_next_type0_request(&port, 0x00U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, 0x03U);
    assert_next_type0_request(&port, 0x00U);
    assert_next_type0_request(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x04U));
    assert_next_type0_request(&port, IOLINK_CMD_PARAM_DOWNLOAD_END);

    feed_type0_byte(&port, (uint8_t)(IOLINK_ISDU_CTRL_START | IOLINK_ISDU_CTRL_LAST));
    feed_type0_byte(&port, 0x00U);
    assert_int_equal(iolink_master_write_parameter_block(&port,
                                                        0x0040U,
                                                        0x01U,
                                                        value,
                                                        sizeof(value)),
                     IOLINK_MASTER_STATUS_PENDING);

    assert_next_type0_request(&port, IOLINK_ISDU_CTRL_START);
    assert_next_type0_request(&port, IOLINK_ISDU_SERVICE_READ << 4);
    assert_next_type0_request(&port, 0x01U);
    assert_next_type0_request(&port, 0x00U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, 0x40U);
    assert_next_type0_request(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x03U));
    assert_next_type0_request(&port, 0x01U);

    feed_type0_byte(&port, IOLINK_ISDU_CTRL_START);
    feed_type0_byte(&port, 0x12U);
    feed_type0_byte(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x01U));
    feed_type0_byte(&port, 0x34U);
    assert_int_equal(iolink_master_write_parameter_block(&port,
                                                        0x0040U,
                                                        0x01U,
                                                        value,
                                                        sizeof(value)),
                     IOLINK_MASTER_STATUS_OK);
    assert_int_equal(iolink_master_get_diagnostics(&port, &diagnostics), 0);
    assert_int_equal(diagnostics.last_service_result, IOLINK_MASTER_STATUS_OK);
}

static void test_public_parameter_block_write_reports_readback_mismatch(void** state)
{
    iolink_master_port_t port;
    iolink_master_diagnostics_t diagnostics;
    const uint8_t value[] = {0x12U, 0x34U};

    (void)state;

    enter_type0_operate(&port);

    assert_int_equal(iolink_master_write_parameter_block(&port,
                                                        0x0040U,
                                                        0x01U,
                                                        value,
                                                        sizeof(value)),
                     IOLINK_MASTER_STATUS_PENDING);
    assert_int_equal(iolink_master_write_parameter_block(&port,
                                                        0x0040U,
                                                        0x02U,
                                                        value,
                                                        sizeof(value)),
                     IOLINK_MASTER_ISDU_ERR_BUSY);

    assert_next_type0_request(&port, IOLINK_ISDU_CTRL_START);
    assert_next_type0_request(&port, (uint8_t)((IOLINK_ISDU_SERVICE_WRITE << 4) | 1U));
    assert_next_type0_request(&port, 0x01U);
    assert_next_type0_request(&port, 0x00U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, 0x03U);
    assert_next_type0_request(&port, 0x00U);
    assert_next_type0_request(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x04U));
    assert_next_type0_request(&port, IOLINK_CMD_PARAM_DOWNLOAD_START);

    feed_type0_byte(&port, (uint8_t)(IOLINK_ISDU_CTRL_START | IOLINK_ISDU_CTRL_LAST));
    feed_type0_byte(&port, 0x00U);
    assert_int_equal(iolink_master_write_parameter_block(&port,
                                                        0x0040U,
                                                        0x01U,
                                                        value,
                                                        sizeof(value)),
                     IOLINK_MASTER_STATUS_PENDING);
    assert_next_type0_request(&port, IOLINK_ISDU_CTRL_START);
    assert_next_type0_request(&port, (uint8_t)((IOLINK_ISDU_SERVICE_WRITE << 4) | 2U));
    assert_next_type0_request(&port, 0x01U);
    assert_next_type0_request(&port, 0x00U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, 0x40U);
    assert_next_type0_request(&port, 0x03U);
    assert_next_type0_request(&port, 0x01U);
    assert_next_type0_request(&port, 0x04U);
    assert_next_type0_request(&port, 0x12U);
    assert_next_type0_request(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x05U));
    assert_next_type0_request(&port, 0x34U);

    feed_type0_byte(&port, (uint8_t)(IOLINK_ISDU_CTRL_START | IOLINK_ISDU_CTRL_LAST));
    feed_type0_byte(&port, 0x00U);
    assert_int_equal(iolink_master_write_parameter_block(&port,
                                                        0x0040U,
                                                        0x01U,
                                                        value,
                                                        sizeof(value)),
                     IOLINK_MASTER_STATUS_PENDING);
    assert_next_type0_request(&port, IOLINK_ISDU_CTRL_START);
    assert_next_type0_request(&port, (uint8_t)((IOLINK_ISDU_SERVICE_WRITE << 4) | 1U));
    assert_next_type0_request(&port, 0x01U);
    assert_next_type0_request(&port, 0x00U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, 0x03U);
    assert_next_type0_request(&port, 0x00U);
    assert_next_type0_request(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x04U));
    assert_next_type0_request(&port, IOLINK_CMD_PARAM_DOWNLOAD_END);

    feed_type0_byte(&port, (uint8_t)(IOLINK_ISDU_CTRL_START | IOLINK_ISDU_CTRL_LAST));
    feed_type0_byte(&port, 0x00U);
    assert_int_equal(iolink_master_write_parameter_block(&port,
                                                        0x0040U,
                                                        0x01U,
                                                        value,
                                                        sizeof(value)),
                     IOLINK_MASTER_STATUS_PENDING);
    assert_next_type0_request(&port, IOLINK_ISDU_CTRL_START);
    assert_next_type0_request(&port, IOLINK_ISDU_SERVICE_READ << 4);
    assert_next_type0_request(&port, 0x01U);
    assert_next_type0_request(&port, 0x00U);
    assert_next_type0_request(&port, 0x02U);
    assert_next_type0_request(&port, 0x40U);
    assert_next_type0_request(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x03U));
    assert_next_type0_request(&port, 0x01U);

    feed_type0_byte(&port, IOLINK_ISDU_CTRL_START);
    feed_type0_byte(&port, 0x12U);
    feed_type0_byte(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x01U));
    feed_type0_byte(&port, 0x35U);
    assert_int_equal(iolink_master_write_parameter_block(&port,
                                                        0x0040U,
                                                        0x01U,
                                                        value,
                                                        sizeof(value)),
                     IOLINK_MASTER_ISDU_ERR_VERIFY_FAILED);
    assert_int_equal(iolink_master_get_diagnostics(&port, &diagnostics), 0);
    assert_int_equal(diagnostics.last_service_result,
                     IOLINK_MASTER_ISDU_ERR_VERIFY_FAILED);

    assert_int_equal(iolink_master_write_parameter_block(&port,
                                                        0x0040U,
                                                        0x02U,
                                                        value,
                                                        sizeof(value)),
                     IOLINK_MASTER_STATUS_PENDING);
}

int main(void)
{
    const struct CMUnitTest tests[] = {
        cmocka_unit_test_setup(test_public_type0_isdu_read_completes_without_private_state,
                               reset_fixture),
        cmocka_unit_test_setup(test_public_data_storage_read_uses_standard_index,
                               reset_fixture),
        cmocka_unit_test_setup(test_public_detailed_device_status_read_uses_standard_index,
                               reset_fixture),
        cmocka_unit_test_setup(test_public_event_code_read_uses_standard_index_and_decodes,
                               reset_fixture),
        cmocka_unit_test_setup(test_public_event_ack_reads_and_returns_event_code,
                               reset_fixture),
        cmocka_unit_test_setup(test_public_event_details_read_decodes_detailed_device_status,
                               reset_fixture),
        cmocka_unit_test_setup(test_public_isdu_verify_readback_compares_value,
                               reset_fixture),
        cmocka_unit_test_setup(test_public_data_storage_verify_uses_standard_index,
                               reset_fixture),
        cmocka_unit_test_setup(test_public_parameter_download_helpers_write_system_commands,
                               reset_fixture),
        cmocka_unit_test_setup(test_public_parameter_upload_and_store_helpers_write_system_commands,
                               reset_fixture),
        cmocka_unit_test_setup(
            test_public_parameter_block_write_sequences_commands_and_readback,
            reset_fixture),
        cmocka_unit_test_setup(test_public_parameter_block_write_reports_readback_mismatch,
                               reset_fixture),
    };

    return cmocka_run_group_tests(tests, NULL, NULL);
}
