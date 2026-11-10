#include <setjmp.h>
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#include <cmocka.h>

#include "iolinki/protocol.h"
#include "iolinki_master/master.h"
#include "../src/master_internal.h"

/*
 * Byte-exact wire conformance for the master ISDU transport.
 *
 * Every expected octet below is computed from the normative Python oracle in the
 * implementation plan (A.1.6 message checksum, A.5.3 Length/ExtLength, A.5.6
 * CHKPDU, Table 52 FlowCTRL) and is written here as a literal. The M-sequence
 * framing is A.1.2: MC carries the channel and FlowCTRL, CKT carries the
 * A.1.6 checksum in bits 0-5, and the ISDU request stream travels as the OD.
 */

/** @brief A.1.6 message checksum (oracle from the plan; not the implementation). */
static uint8_t wire_ck6(const uint8_t* octets, size_t len)
{
    uint8_t c = 0x52U;
    uint8_t b[8];
    size_t i;
    uint8_t r = 0U;

    for (i = 0U; i < len; i++) {
        c ^= octets[i];
    }
    for (i = 0U; i < 8U; i++) {
        b[i] = (uint8_t) ((c >> i) & 0x01U);
    }
    r |= (uint8_t) ((b[7] ^ b[5] ^ b[3] ^ b[1]) << 5U);
    r |= (uint8_t) ((b[6] ^ b[4] ^ b[2] ^ b[0]) << 4U);
    r |= (uint8_t) ((b[7] ^ b[6]) << 3U);
    r |= (uint8_t) ((b[5] ^ b[4]) << 2U);
    r |= (uint8_t) ((b[3] ^ b[2]) << 1U);
    r |= (uint8_t) (b[1] ^ b[0]);
    return r;
}

/** @brief A.5.6 CHKPDU: XOR of every ISDU octet with CHKPDU still zero. */
static uint8_t wire_chkpdu(const uint8_t* octets, uint16_t len)
{
    uint8_t c = 0U;
    uint16_t i;

    for (i = 0U; i < len; i++) {
        c ^= octets[i];
    }
    return c;
}

static int g_send_calls;
static uint8_t g_sent[128][64];
static size_t g_sent_len[128];

static int wire_send(void* user, const uint8_t* data, size_t len)
{
    (void) user;
    assert_non_null(data);
    assert_in_range(g_send_calls, 0, 127);
    assert_in_range(len, 1U, sizeof(g_sent[0]));

    memcpy(g_sent[g_send_calls], data, len);
    g_sent_len[g_send_calls] = len;
    g_send_calls++;
    return (int) len;
}

static const iolink_phy_api_t g_phy = {
    .send = wire_send,
};

static const iolink_master_config_t g_config = {
    .port_mode = IOLINK_MASTER_PORT_MODE_IOLINK,
    .m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_0,
    .baudrate = IOLINK_BAUDRATE_COM3,
    .min_cycle_time = 20U,
};

static int reset_wire(void** state)
{
    (void) state;
    g_send_calls = 0;
    memset(g_sent, 0, sizeof(g_sent));
    memset(g_sent_len, 0, sizeof(g_sent_len));
    return 0;
}

static void feed_type0_byte(iolink_master_port_t* port, uint8_t byte)
{
    uint8_t frame[2];

    frame[0] = byte;
    frame[1] = wire_ck6(frame, 1U); /* Over [byte, CKS=0], event/PD flags zero. */
    assert_int_equal(iolink_master_on_rx(port, frame, sizeof(frame)), IOLINK_MASTER_STATUS_OK);
}

static void feed_operate_ack(iolink_master_port_t* port)
{
    static const uint8_t ack[1] = {0x2DU};

    assert_int_equal(iolink_master_on_rx(port, ack, sizeof(ack)), IOLINK_MASTER_STATUS_OK);
}

static void enter_operate_cfg(iolink_master_port_t* port,
                             const iolink_master_config_t* config)
{
    assert_int_equal(iolink_master_init(port, &g_phy, config), IOLINK_MASTER_STATUS_OK);
    assert_int_equal(iolink_master_tick_event(port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_int_equal(iolink_master_tick_event(port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    feed_type0_byte(port, 0x00U);
    assert_int_equal(iolink_master_tick_event(port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    feed_operate_ack(port);
    assert_int_equal(iolink_master_get_state(port), IOLINK_MASTER_STATE_OPERATE);
}

static void enter_type0_operate(iolink_master_port_t* port)
{
    enter_operate_cfg(port, &g_config);
}

/** @brief Assert the last emitted frame equals @p expected byte for byte. */
static void assert_last_frame(const uint8_t* expected, size_t len)
{
    assert_true(g_send_calls > 0);
    assert_int_equal(g_sent_len[g_send_calls - 1], len);
    assert_memory_equal(g_sent[g_send_calls - 1], expected, len);
}

/** @brief Drive tick until the request phase completes (T5 entered). */
static void drain_request_wire(iolink_master_port_t* port)
{
    uint16_t guard = 0U;

    while (iolink_master_port_state(port)->isdu.phase == IOLINK_MASTER_ISDU_PHASE_REQUEST) {
        assert_int_equal(iolink_master_tick_event(port, IOLINK_MASTER_TICK_CYCLE_DUE),
                         IOLINK_MASTER_STATUS_OK);
        guard++;
        assert_true(guard < 256U);
    }
}

static void test_wire_8bit_read_request_frames(void** state)
{
    iolink_master_port_t port;
    uint8_t data[4] = {0U};
    uint8_t len = sizeof(data);
    /* Oracle: A.1.6 over [MC, CKT=0, OD]; MC = R|ISDU|FlowCTRL (A.1.2, Table 52). */
    static const uint8_t f0[] = {0x70U, 0x09U, 0x93U}; /* START, OD = read req hdr */
    static const uint8_t f1[] = {0x61U, 0x14U, 0x10U}; /* COUNT 1, Index 0x10 */
    static const uint8_t f2[] = {0x62U, 0x28U, 0x83U}; /* COUNT 2, CHKPDU */
    static const uint8_t poll[] = {0xF0U, 0x2DU};

    (void) state;

    enter_type0_operate(&port);
    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, data, &len), 1);

    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_last_frame(f0, sizeof(f0));

    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_last_frame(f1, sizeof(f1));

    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_last_frame(f2, sizeof(f2));

    /* T5: request sent, the master polls with a read and FlowCTRL START (Table 52). */
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_last_frame(poll, sizeof(poll));
}

static void test_wire_16bit_read_request_frames(void** state)
{
    iolink_master_port_t port;
    uint8_t data[4] = {0U};
    uint8_t len = sizeof(data);
    /* Oracle: request stream B5 12 34 56 C5 (Table A.13/A.15, A.5.6). */
    static const uint8_t f0[] = {0x70U, 0x1EU, 0xB5U};
    static const uint8_t f1[] = {0x61U, 0x35U, 0x12U};
    static const uint8_t f2[] = {0x62U, 0x12U, 0x34U};
    static const uint8_t f3[] = {0x63U, 0x1EU, 0x56U};
    static const uint8_t f4[] = {0x64U, 0x30U, 0xC5U};

    (void) state;

    enter_type0_operate(&port);
    assert_int_equal(iolink_master_read_isdu(&port, 0x1234U, 0x56U, data, &len), 1);

    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_last_frame(f0, sizeof(f0));
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_last_frame(f1, sizeof(f1));
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_last_frame(f2, sizeof(f2));
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_last_frame(f3, sizeof(f3));
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_last_frame(f4, sizeof(f4));
}

static void test_wire_write_request_and_positive_ack(void** state)
{
    iolink_master_port_t port;
    static const uint8_t payload[] = {0x12U, 0x34U};
    /* Oracle: request stream 26 10 01 12 34 11; ack stream 52 52 (Table A.13). */
    static const uint8_t f0[] = {0x70U, 0x12U, 0x26U};
    static const uint8_t f1[] = {0x61U, 0x14U, 0x10U};
    static const uint8_t f2[] = {0x62U, 0x21U, 0x01U};
    static const uint8_t f3[] = {0x63U, 0x14U, 0x12U};
    static const uint8_t f4[] = {0x64U, 0x21U, 0x34U};
    static const uint8_t f5[] = {0x65U, 0x17U, 0x11U};

    (void) state;

    enter_type0_operate(&port);
    assert_int_equal(iolink_master_write_isdu(&port, 0x0010U, 0x01U, payload, sizeof(payload)), 1);

    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_last_frame(f0, sizeof(f0));
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_last_frame(f1, sizeof(f1));
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_last_frame(f2, sizeof(f2));
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_last_frame(f3, sizeof(f3));
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_last_frame(f4, sizeof(f4));
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_last_frame(f5, sizeof(f5));

    /* The request is fully sent (T5); deliver the Write Response (+) stream 52 52. */
    feed_type0_byte(&port, 0x52U);
    feed_type0_byte(&port, 0x52U);

    assert_int_equal(iolink_master_write_isdu(&port, 0x0010U, 0x01U, payload, sizeof(payload)), 0);
}

static void test_wire_negative_response_maps_error_type(void** state)
{
    iolink_master_port_t port;
    iolink_master_diagnostics_t diagnostics;
    uint8_t data[4] = {0U};
    uint8_t len = sizeof(data);
    /* Oracle: C4 80 11 55 -> ErrorType 0x8011 (Table A.13, A.5.6). */

    (void) state;

    enter_type0_operate(&port);
    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, data, &len), 1);
    while (iolink_master_port_state(&port)->isdu.phase == IOLINK_MASTER_ISDU_PHASE_REQUEST) {
        assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                         IOLINK_MASTER_STATUS_OK);
    }

    feed_type0_byte(&port, 0xC4U);
    feed_type0_byte(&port, 0x80U);
    feed_type0_byte(&port, 0x11U);
    feed_type0_byte(&port, 0x55U);

    len = sizeof(data);
    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, data, &len), -4);
    assert_int_equal(iolink_master_get_diagnostics(&port, &diagnostics), 0);
    assert_int_equal(diagnostics.last_isdu_error, 0x8011U);
}

static void test_wire_64_octet_read_uses_extended_length(void** state)
{
    iolink_master_port_t port;
    uint8_t data[80] = {0U};
    uint8_t len = sizeof(data);
    uint8_t stream[80] = {0U};
    uint8_t payload[64];
    uint16_t total;
    uint16_t i;
    uint8_t chk;

    (void) state;

    for (i = 0U; i < sizeof(payload); i++) {
        payload[i] = (uint8_t) i;
    }

    /* Read Response (+): D1 ExtLength(67) payload... CHKPDU (A.5.3, Table A.14). */
    total = 67U;
    stream[0] = 0xD1U;
    stream[1] = 67U;
    memcpy(&stream[2], payload, sizeof(payload));
    chk = wire_chkpdu(stream, 66U);
    stream[66] = chk;

    enter_type0_operate(&port);
    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, data, &len), 1);

    drain_request_wire(&port);
    for (i = 0U; i < total; i++) {
        feed_type0_byte(&port, stream[i]);
    }

    len = sizeof(data);
    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, data, &len), 0);
    assert_int_equal(len, 64U);
    assert_memory_equal(data, payload, sizeof(payload));
}

/** @brief Drive one DIAGNOSIS read of @p mc and answer it with @p value.
 *
 * Verifies the emitted TYPE_0 read M-sequence byte for byte (A.1.6 over
 * [MC, CKT=0]) and injects the A.1.5 reply [value, CKS].
 */
static void serve_diagnosis_read(iolink_master_port_t* port, uint8_t mc, uint8_t value)
{
    uint8_t frame[2];

    assert_int_equal(iolink_master_tick_event(port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    frame[0] = mc;
    frame[1] = wire_ck6(frame, 1U);
    assert_last_frame(frame, sizeof(frame));
    feed_type0_byte(port, value);
}

static void test_wire_event_details_reads_diagnosis_memory(void** state)
{
    iolink_master_port_t port;
    iolink_master_event_t events[2];
    uint8_t count = 0U;
    /* Oracle: MC = R|DIAGNOSIS|addr; memory = StatusCode 0x01 + slot1 E2 42 10. */
    static const uint8_t mc0[] = {0xC0U, 0x1DU};
    static const uint8_t mc1[] = {0xC1U, 0x0CU};
    static const uint8_t mc2[] = {0xC2U, 0x3CU};
    static const uint8_t mc3[] = {0xC3U, 0x2DU};

    (void) state;

    memset(events, 0, sizeof(events));
    enter_type0_operate(&port);

    assert_int_equal(iolink_master_read_event_details(&port, events, 2U, &count),
                     IOLINK_MASTER_STATUS_PENDING);

    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_last_frame(mc0, sizeof(mc0));
    feed_type0_byte(&port, 0x01U);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_last_frame(mc1, sizeof(mc1));
    feed_type0_byte(&port, 0xE2U);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_last_frame(mc2, sizeof(mc2));
    feed_type0_byte(&port, 0x42U);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_last_frame(mc3, sizeof(mc3));
    feed_type0_byte(&port, 0x10U);

    assert_int_equal(iolink_master_read_event_details(&port, events, 2U, &count),
                     IOLINK_MASTER_STATUS_OK);
    assert_int_equal(count, 1U);
    assert_int_equal(events[0].qualifier, 0xE2U);
    assert_int_equal(events[0].type, IOLINK_MASTER_EVENT_TYPE_WARNING);
    assert_int_equal(events[0].code, 0x4210U);
}

static void test_wire_event_ack_writes_status_code(void** state)
{
    iolink_master_port_t port;
    uint16_t event_code = 0U;
    /* Oracle: Table 59 T8 confirm write MC = W|DIAGNOSIS|0, OD any (0x00). */
    static const uint8_t confirm[] = {0x40U, 0x35U, 0x00U};

    (void) state;

    enter_type0_operate(&port);
    assert_int_equal(iolink_master_ack_event(&port, &event_code), IOLINK_MASTER_STATUS_PENDING);

    serve_diagnosis_read(&port, 0xC0U, 0x01U);
    serve_diagnosis_read(&port, 0xC1U, 0xE2U);
    serve_diagnosis_read(&port, 0xC2U, 0x42U);
    serve_diagnosis_read(&port, 0xC3U, 0x10U);

    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_last_frame(confirm, sizeof(confirm));

    assert_int_equal(iolink_master_ack_event(&port, &event_code), IOLINK_MASTER_STATUS_OK);
    assert_int_equal(event_code, 0x4210U);
}

static void test_wire_2v_read_poll_has_no_od(void** state)
{
    iolink_master_port_t port;
    uint8_t data[4] = {0U};
    uint8_t len = sizeof(data);
    iolink_master_config_t config = g_config;
    uint8_t poll[4] = {0xF0U, IOLINK_MSEQ_TYPE_2, 0x00U, 0x00U};

    (void) state;

    config.m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_2_V;
    config.pd_in_len = 2U;
    config.pd_out_len = 2U;
    enter_operate_cfg(&port, &config);
    assert_int_equal(iolink_master_read_isdu(&port, 0x0010U, 0U, data, &len), 1);

    /* Three write frames carry the 3-octet ISDU read request, then the poll is
       a read M-sequence: MC + CKT + PD-out(2); a read request carries no OD. */
    for (int i = 0; i < 3; i++) {
        assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                         IOLINK_MASTER_STATUS_OK);
    }
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    poll[1] = (uint8_t) (poll[1] | wire_ck6(poll, sizeof(poll)));
    assert_last_frame(poll, sizeof(poll));
}

static void test_wire_1v_write_request_uses_eight_od_octets(void** state)
{
    iolink_master_port_t port;
    const uint8_t payload[1] = {0x12U};
    iolink_master_config_t config = g_config;

    (void) state;

    config.m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_1_V;
    config.pd_in_len = 0U;
    config.pd_out_len = 0U;
    enter_operate_cfg(&port, &config);
    assert_int_equal(iolink_master_write_isdu(&port, 0x0010U, 0x01U, payload, sizeof(payload)), 1);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_int_equal(g_sent_len[g_send_calls - 1], 2U + 8U); /* MC+CKT+OD(8) */
}

static void test_wire_1_2_write_request_uses_two_od_octets(void** state)
{
    iolink_master_port_t port;
    const uint8_t payload[1] = {0x12U};
    iolink_master_config_t config = g_config;

    (void) state;

    config.m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_1_2;
    config.pd_in_len = 0U;
    config.pd_out_len = 0U;
    enter_operate_cfg(&port, &config);
    assert_int_equal(iolink_master_write_isdu(&port, 0x0010U, 0x01U, payload, sizeof(payload)), 1);
    assert_int_equal(iolink_master_tick_event(&port, IOLINK_MASTER_TICK_CYCLE_DUE),
                     IOLINK_MASTER_STATUS_OK);
    assert_int_equal(g_sent_len[g_send_calls - 1], 2U + 2U); /* MC+CKT+OD(2) */
}

int main(void)
{
    const struct CMUnitTest tests[] = {
        cmocka_unit_test_setup(test_wire_8bit_read_request_frames, reset_wire),
        cmocka_unit_test_setup(test_wire_16bit_read_request_frames, reset_wire),
        cmocka_unit_test_setup(test_wire_write_request_and_positive_ack, reset_wire),
        cmocka_unit_test_setup(test_wire_negative_response_maps_error_type, reset_wire),
        cmocka_unit_test_setup(test_wire_64_octet_read_uses_extended_length, reset_wire),
        cmocka_unit_test_setup(test_wire_event_details_reads_diagnosis_memory, reset_wire),
        cmocka_unit_test_setup(test_wire_event_ack_writes_status_code, reset_wire),
        cmocka_unit_test_setup(test_wire_2v_read_poll_has_no_od, reset_wire),
        cmocka_unit_test_setup(test_wire_1v_write_request_uses_eight_od_octets, reset_wire),
        cmocka_unit_test_setup(test_wire_1_2_write_request_uses_two_od_octets, reset_wire),
    };

    return cmocka_run_group_tests(tests, NULL, NULL);
}
