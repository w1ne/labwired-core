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

static int g_send_calls;
static int g_forced_send_return;
static uint8_t g_sent[16][64];
static size_t g_sent_len[16];

static int fake_phy_send(void* user, const uint8_t* data, size_t len)
{
    (void)user;
    assert_non_null(data);
    assert_in_range(len, 1U, sizeof(g_sent[0]));
    assert_in_range(g_send_calls, 0, 15);

    memcpy(g_sent[g_send_calls], data, len);
    g_sent_len[g_send_calls] = len;
    g_send_calls++;

    if(g_forced_send_return != INT_MIN)
    {
        return g_forced_send_return;
    }

    return (int)len;
}

static const iolink_phy_api_t g_fake_phy = {
    .send = fake_phy_send,
};

static const iolink_master_config_t g_config = {
    .m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_2_1,
    .baudrate = IOLINK_BAUDRATE_COM3,
    .min_cycle_time = 20U,
    .pd_in_len = 0U,
    .pd_out_len = 0U,
};

static int reset_fake_phy(void** state)
{
    (void)state;
    g_send_calls = 0;
    g_forced_send_return = INT_MIN;
    memset(g_sent, 0, sizeof(g_sent));
    memset(g_sent_len, 0, sizeof(g_sent_len));
    return 0;
}

static void enter_operate(iolink_master_port_t* port)
{
    uint8_t startup_resp[2] = {0U};

    assert_int_equal(iolink_master_init(port, &g_fake_phy, &g_config), 0);

    iolink_master_process(port);
    iolink_master_process(port);
    startup_resp[1] = iolink_checksum_ck(startup_resp[0], 0U);
    assert_int_equal(iolink_master_on_rx(port, startup_resp, sizeof(startup_resp)), 0);
    iolink_master_process(port);

    assert_int_equal(iolink_master_get_state(port), IOLINK_MASTER_STATE_OPERATE);
    assert_int_equal(g_send_calls, 3);
}

static void assert_last_od(uint8_t od0, uint8_t od1)
{
    assert_true(g_send_calls > 0);
    assert_int_equal(g_sent_len[g_send_calls - 1], 5U);
    assert_int_equal(g_sent[g_send_calls - 1][2], od0);
    assert_int_equal(g_sent[g_send_calls - 1][3], od1);
}

static void feed_response_od(iolink_master_port_t* port, uint8_t od0, uint8_t od1)
{
    uint8_t frame[4];

    frame[0] = IOLINK_OD_STATUS_PD_VALID;
    frame[1] = od0;
    frame[2] = od1;
    frame[3] = iolink_crc6(frame, 3U);

    assert_int_equal(iolink_master_on_rx(port, frame, sizeof(frame)), 0);
}

static void feed_isdu_response_bytes(iolink_master_port_t* port, const uint8_t* data, uint8_t len)
{
    uint8_t i;
    uint8_t ctrl;

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
        feed_response_od(port, ctrl, data[i]);
    }
}

static void feed_type0_isdu_response_bytes(iolink_master_port_t* port,
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

static void test_read_isdu_rejects_invalid_args(void** state)
{
    iolink_master_port_t port = {0};
    uint8_t data[8] = {0U};
    uint8_t len = sizeof(data);

    (void)state;

    assert_int_equal(iolink_master_read_isdu(NULL, 0x0010U, 0U, data, &len), -1);
    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, NULL, &len), -1);
    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, data, NULL), -1);
}

static void test_read_isdu_returns_pending_for_valid_request(void** state)
{
    iolink_master_port_t port;
    uint8_t data[8] = {0U};
    uint8_t len = sizeof(data);

    (void)state;

    enter_operate(&port);

    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, data, &len), 1);
}

static void test_read_isdu_rejects_non_operate_state(void** state)
{
    iolink_master_port_t port = {0};
    uint8_t data[8] = {0U};
    uint8_t len = sizeof(data);

    (void)state;

    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, data, &len), -5);
}

static void test_read_isdu_emits_segmented_request_bytes(void** state)
{
    iolink_master_port_t port;
    uint8_t data[8] = {0U};
    uint8_t len = sizeof(data);

    (void)state;

    enter_operate(&port);

    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, data, &len), 1);

    iolink_master_process(&port);
    assert_last_od(IOLINK_ISDU_CTRL_START, IOLINK_ISDU_SERVICE_READ << 4);

    iolink_master_process(&port);
    assert_last_od(0x01U, 0x00U);

    iolink_master_process(&port);
    assert_last_od(0x02U, 0x10U);

    iolink_master_process(&port);
    assert_last_od((uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x03U), 0x00U);
}

static void test_type0_read_isdu_emits_one_byte_type0_frames(void** state)
{
    iolink_master_port_t port = {0};
    uint8_t data[8] = {0U};
    uint8_t len = sizeof(data);
    uint8_t expected[2] = {0U};
    int expected_len;

    (void)state;

    iolink_master_port_state(&port)->phy = &g_fake_phy;
    iolink_master_port_state(&port)->state = IOLINK_MASTER_STATE_OPERATE;
    iolink_master_port_state(&port)->config.m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_0;
    iolink_master_port_state(&port)->od_len = 1U;

    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, data, &len), 1);

    iolink_master_process(&port);
    expected_len = iolink_frame_encode_type0(IOLINK_ISDU_CTRL_START, expected, sizeof(expected));
    assert_int_equal(expected_len, 2);
    assert_int_equal(g_send_calls, 1);
    assert_int_equal(g_sent_len[0], (size_t)expected_len);
    assert_memory_equal(g_sent[0], expected, (size_t)expected_len);

    iolink_master_process(&port);
    expected_len = iolink_frame_encode_type0((uint8_t)(IOLINK_ISDU_SERVICE_READ << 4),
                                             expected,
                                             sizeof(expected));
    assert_int_equal(expected_len, 2);
    assert_int_equal(g_send_calls, 2);
    assert_int_equal(g_sent_len[1], (size_t)expected_len);
    assert_memory_equal(g_sent[1], expected, (size_t)expected_len);
}

static void test_type0_read_isdu_completes_from_type0_response_bytes(void** state)
{
    iolink_master_port_t port = {0};
    uint8_t data[8] = {0U};
    uint8_t len = sizeof(data);
    uint8_t frame[2] = {0U};

    (void)state;

    iolink_master_port_state(&port)->phy = &g_fake_phy;
    iolink_master_port_state(&port)->state = IOLINK_MASTER_STATE_OPERATE;
    iolink_master_port_state(&port)->config.m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_0;
    iolink_master_port_state(&port)->od_len = 1U;

    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, data, &len), 1);

    frame[0] = IOLINK_ISDU_CTRL_START;
    frame[1] = iolink_checksum_ck(frame[0], 0U);
    assert_int_equal(iolink_master_on_rx(&port, frame, sizeof(frame)), 0);

    frame[0] = 0x4FU;
    frame[1] = iolink_checksum_ck(frame[0], 0U);
    assert_int_equal(iolink_master_on_rx(&port, frame, sizeof(frame)), 0);

    frame[0] = (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x01U);
    frame[1] = iolink_checksum_ck(frame[0], 0U);
    assert_int_equal(iolink_master_on_rx(&port, frame, sizeof(frame)), 0);

    frame[0] = 0x4BU;
    frame[1] = iolink_checksum_ck(frame[0], 0U);
    assert_int_equal(iolink_master_on_rx(&port, frame, sizeof(frame)), 0);

    len = sizeof(data);
    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, data, &len), 0);
    assert_int_equal(len, 2U);
    assert_int_equal(data[0], 0x4FU);
    assert_int_equal(data[1], 0x4BU);
}

static void test_read_isdu_completes_after_response_bytes(void** state)
{
    iolink_master_port_t port;
    uint8_t data[8] = {0U};
    uint8_t len = sizeof(data);

    (void)state;

    enter_operate(&port);
    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, data, &len), 1);

    feed_response_od(&port, IOLINK_ISDU_CTRL_START, 0x4FU);
    feed_response_od(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x01U), 0x4BU);

    len = sizeof(data);
    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, data, &len), 0);
    assert_int_equal(len, 2U);
    assert_int_equal(data[0], 0x4FU);
    assert_int_equal(data[1], 0x4BU);
}

static void test_read_isdu_skips_pre_response_filler_after_request_sent(void** state)
{
    iolink_master_port_t port;
    uint8_t data[8] = {0U};
    uint8_t len = sizeof(data);

    (void)state;

    enter_operate(&port);
    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, data, &len),
                     IOLINK_MASTER_STATUS_PENDING);

    /*
     * Regression (found by the LabWired full-flow model): once the request is
     * fully transmitted, a real device needs one or more idle cycles to compute
     * its response and emits filler 0x00 OD bytes in the meantime. The response
     * collector must skip those, not consume the first 0x00 as the response
     * START control byte — otherwise the control/data phase desyncs and every
     * following sequence number mismatches (SEGMENTATION error). The skip gate
     * must not depend on request_sent.
     */
    iolink_master_port_state(&port)->isdu.request_sent = true;

    feed_response_od(&port, 0x00U, 0x00U);
    feed_response_od(&port, 0x00U, 0x00U);

    /* The real single-segment response then completes the read cleanly. */
    feed_response_od(&port, (uint8_t)(IOLINK_ISDU_CTRL_START | IOLINK_ISDU_CTRL_LAST), 0x4FU);

    len = sizeof(data);
    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, data, &len),
                     IOLINK_MASTER_STATUS_OK);
    assert_int_equal(len, 1U);
    assert_int_equal(data[0], 0x4FU);
}

static void test_read_device_info_reads_direct_parameter_page1(void** state)
{
    static const uint8_t page1[] = {
        0x00U,
        0x00U,
        10U,
        0x01U,
        0x11U,
        0x00U,
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
    iolink_master_port_t port;
    iolink_master_device_info_t info;

    (void)state;

    enter_operate(&port);

    assert_int_equal(iolink_master_read_device_info(&port), 1);
    feed_isdu_response_bytes(&port, page1, sizeof(page1));
    assert_int_equal(iolink_master_read_device_info(&port), 0);
    assert_int_equal(iolink_master_get_device_info(&port, &info), 0);
    assert_int_equal(info.vendor_id, 0x1234U);
    assert_int_equal(info.device_id, 0x56789AU);
}

static void test_read_device_info_rejects_incompatible_device_page(void** state)
{
    uint8_t page1[16] = {0U};
    iolink_master_port_t port;

    (void)state;

    page1[0x02] = 10U;
    page1[0x03] = 0x03U;
    page1[0x04] = 0x11U;

    enter_operate(&port);

    assert_int_equal(iolink_master_read_device_info(&port), 1);
    feed_isdu_response_bytes(&port, page1, sizeof(page1));
    assert_int_equal(iolink_master_read_device_info(&port), -5);
}

static void test_preoperate_read_device_info_uses_type0_parameter_frames(void** state)
{
    static const uint8_t page1[] = {
        0x00U,
        0x00U,
        10U,
        0x01U,
        0x11U,
        0x00U,
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
    iolink_master_port_t port;
    iolink_master_device_info_t info;
    uint8_t startup_resp[2] = {0U};
    uint8_t transition[8] = {0U};
    int expected_len;

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_fake_phy, &g_config), 0);
    iolink_master_process(&port);
    iolink_master_process(&port);
    startup_resp[1] = iolink_checksum_ck(startup_resp[0], 0U);
    assert_int_equal(iolink_master_on_rx(&port, startup_resp, sizeof(startup_resp)), 0);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_PREOPERATE);

    assert_int_equal(iolink_master_read_device_info(&port), 1);
    iolink_master_process(&port);
    assert_int_equal(g_sent_len[g_send_calls - 1], 2U);
    assert_int_equal(g_sent[g_send_calls - 1][0], IOLINK_ISDU_CTRL_START);

    feed_type0_isdu_response_bytes(&port, page1, sizeof(page1));
    assert_int_equal(iolink_master_read_device_info(&port), 0);
    assert_int_equal(iolink_master_get_device_info(&port, &info), 0);
    assert_int_equal(info.vendor_id, 0x1234U);

    iolink_master_process(&port);
    /* Transition to OPERATE is a Type-0 WRITE of MasterCommand DeviceOperate
       (0x99) to Direct Parameter address 0x00 on the page channel (MC 0x20). */
    expected_len = iolink_frame_encode_type0_write(
        iolink_master_encode_master_command(false, IOLINK_MASTER_MC_CHANNEL_PAGE,
                                            IOLINK_MASTER_DPP1_OFF_MASTER_COMMAND),
        IOLINK_CMD_DEVICE_OPERATE, transition, sizeof(transition));
    assert_int_equal(expected_len, 3);
    assert_int_equal(g_sent_len[g_send_calls - 1], (size_t)expected_len);
    assert_memory_equal(g_sent[g_send_calls - 1], transition, (size_t)expected_len);
}

static void test_read_isdu_reports_small_result_buffer(void** state)
{
    iolink_master_port_t port;
    uint8_t data[1] = {0U};
    uint8_t len = sizeof(data);

    (void)state;

    enter_operate(&port);
    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, data, &len), 1);

    feed_response_od(&port, IOLINK_ISDU_CTRL_START, 0x4FU);
    feed_response_od(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x01U), 0x4BU);

    len = sizeof(data);
    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, data, &len), -2);
    assert_int_equal(len, 2U);
}

static void test_write_isdu_rejects_invalid_args(void** state)
{
    iolink_master_port_t port = {0};
    const uint8_t data[] = {0x11U, 0x22U};

    (void)state;

    assert_int_equal(iolink_master_write_isdu(NULL, 0x0010U, 0U, data, sizeof(data)), -1);
    assert_int_equal(iolink_master_write_isdu(&port, 0x0010U, 0U, NULL, sizeof(data)), -1);
}

static void test_write_isdu_accepts_valid_and_zero_length_requests(void** state)
{
    iolink_master_port_t port;
    const uint8_t data[] = {0x11U, 0x22U};

    (void)state;

    enter_operate(&port);

    assert_int_equal(iolink_master_write_isdu(&port, 0x0010U, 0U, data, sizeof(data)), 1);

    feed_response_od(&port,
                     (uint8_t)(IOLINK_ISDU_CTRL_START | IOLINK_ISDU_CTRL_LAST),
                     0x00U);
    assert_int_equal(iolink_master_write_isdu(&port, 0x0010U, 0U, data, sizeof(data)), 0);

    assert_int_equal(iolink_master_write_isdu(&port, 0x0010U, 0U, NULL, 0U), 1);
}

static void test_write_isdu_rejects_non_operate_state(void** state)
{
    iolink_master_port_t port = {0};
    const uint8_t data[] = {0x11U, 0x22U};

    (void)state;

    assert_int_equal(iolink_master_write_isdu(&port, 0x0010U, 0U, data, sizeof(data)), -5);
}

static void test_write_isdu_emits_payload_request_bytes(void** state)
{
    iolink_master_port_t port;
    const uint8_t data[] = {0x11U, 0x22U};

    (void)state;

    enter_operate(&port);

    assert_int_equal(iolink_master_write_isdu(&port, 0x0018U, 0U, data, sizeof(data)), 1);

    iolink_master_process(&port);
    assert_last_od(IOLINK_ISDU_CTRL_START,
                   (uint8_t)((IOLINK_ISDU_SERVICE_WRITE << 4) | sizeof(data)));

    iolink_master_process(&port);
    assert_last_od(0x01U, 0x00U);

    iolink_master_process(&port);
    assert_last_od(0x02U, 0x18U);

    iolink_master_process(&port);
    assert_last_od(0x03U, 0x00U);

    iolink_master_process(&port);
    assert_last_od(0x04U, 0x11U);

    iolink_master_process(&port);
    assert_last_od((uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x05U), 0x22U);
}

static void test_write_isdu_completes_after_ack_response(void** state)
{
    iolink_master_port_t port;
    const uint8_t data[] = {0x11U, 0x22U};

    (void)state;

    enter_operate(&port);

    assert_int_equal(iolink_master_write_isdu(&port, 0x0018U, 0U, data, sizeof(data)), 1);

    feed_response_od(&port,
                     (uint8_t)(IOLINK_ISDU_CTRL_START | IOLINK_ISDU_CTRL_LAST),
                     0x00U);

    assert_int_equal(iolink_master_write_isdu(&port, 0x0018U, 0U, data, sizeof(data)), 0);
}

static void test_isdu_rejects_second_request_while_busy(void** state)
{
    iolink_master_port_t port;
    uint8_t data[8] = {0U};
    uint8_t len = sizeof(data);

    (void)state;

    enter_operate(&port);

    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, data, &len), 1);
    assert_int_equal(iolink_master_write_isdu(&port, 0x0018U, 0U, data, 1U), -3);
}

static void test_isdu_response_error_is_reported(void** state)
{
    iolink_master_port_t port;
    iolink_master_diagnostics_t diagnostics;
    uint8_t data[8] = {0U};
    uint8_t len = sizeof(data);

    (void)state;

    enter_operate(&port);
    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, data, &len), 1);

    feed_response_od(&port, IOLINK_ISDU_CTRL_START, 0x80U);
    feed_response_od(&port,
                     (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x01U),
                     IOLINK_ISDU_ERROR_SERVICE_NOT_AVAIL);

    len = sizeof(data);
    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, data, &len), -4);
    assert_int_equal(iolink_master_get_diagnostics(&port, &diagnostics), 0);
    assert_int_equal(diagnostics.last_isdu_error, IOLINK_ISDU_ERROR_SERVICE_NOT_AVAIL);
}

static void test_isdu_response_sequence_error_is_reported(void** state)
{
    iolink_master_port_t port;
    uint8_t data[8] = {0U};
    uint8_t len = sizeof(data);

    (void)state;

    enter_operate(&port);
    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, data, &len), 1);

    feed_response_od(&port, IOLINK_ISDU_CTRL_START, 0x4FU);
    feed_response_od(&port, (uint8_t)(IOLINK_ISDU_CTRL_LAST | 0x02U), 0x4BU);

    len = sizeof(data);
    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, data, &len), -4);
}

int main(void)
{
    const struct CMUnitTest tests[] = {
        cmocka_unit_test_setup(test_read_isdu_rejects_invalid_args, reset_fake_phy),
        cmocka_unit_test_setup(test_read_isdu_returns_pending_for_valid_request, reset_fake_phy),
        cmocka_unit_test_setup(test_read_isdu_rejects_non_operate_state, reset_fake_phy),
        cmocka_unit_test_setup(test_read_isdu_emits_segmented_request_bytes, reset_fake_phy),
        cmocka_unit_test_setup(test_type0_read_isdu_emits_one_byte_type0_frames,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_type0_read_isdu_completes_from_type0_response_bytes,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_read_isdu_completes_after_response_bytes, reset_fake_phy),
        cmocka_unit_test_setup(test_read_isdu_skips_pre_response_filler_after_request_sent,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_read_device_info_reads_direct_parameter_page1, reset_fake_phy),
        cmocka_unit_test_setup(test_read_device_info_rejects_incompatible_device_page,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_preoperate_read_device_info_uses_type0_parameter_frames,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_read_isdu_reports_small_result_buffer, reset_fake_phy),
        cmocka_unit_test_setup(test_write_isdu_rejects_invalid_args, reset_fake_phy),
        cmocka_unit_test_setup(test_write_isdu_accepts_valid_and_zero_length_requests,
                               reset_fake_phy),
        cmocka_unit_test_setup(test_write_isdu_rejects_non_operate_state, reset_fake_phy),
        cmocka_unit_test_setup(test_write_isdu_emits_payload_request_bytes, reset_fake_phy),
        cmocka_unit_test_setup(test_write_isdu_completes_after_ack_response, reset_fake_phy),
        cmocka_unit_test_setup(test_isdu_rejects_second_request_while_busy, reset_fake_phy),
        cmocka_unit_test_setup(test_isdu_response_error_is_reported, reset_fake_phy),
        cmocka_unit_test_setup(test_isdu_response_sequence_error_is_reported, reset_fake_phy),
    };

    return cmocka_run_group_tests(tests, NULL, NULL);
}
