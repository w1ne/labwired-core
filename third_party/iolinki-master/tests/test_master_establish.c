/*
 * Establish-communication tests for the master DL-mode handler.
 *
 * Every expected value below is derived from the IO-Link Interface and System
 * Specification V1.1.5 text, not from this implementation:
 *
 *   - 7.3.2.2, Figure 31, Figure 32, Figure 33: wake-up, then test messages at
 *     COM3, COM2, COM1 in descending order, T_DMT before every message, T_DWU
 *     before a repeated wake-up, T_SD between wake-up retry sequences, PHY
 *     inactive after a failed sequence.
 *   - Figure 36 / Table 44 (EstablishCom_1 submachine, T1..T5, T15..T19):
 *     Retry counter, "Retries < 3" loop, DL_Mode INACTIVE after Retry = 3.
 *   - Table 46 (Inactive_0, AwaitReply_1, T1, T3, T4): the test message is a
 *     TYPE_0 read of MinCycleTime, MC = 0xA2; no answer within T_M-sequence or
 *     an undecodable answer returns to Inactive_0 (no message retry).
 *   - Table 9: T_BIT = 1/4800 s (COM1), 1/38400 s (COM2), 1/230400 s (COM3).
 *   - Table 10: T_REN <= 500 us.
 *   - Table 42: T_DMT 27..37 T_BIT, T_DWU 30..50 ms, n_WU = 2, T_SD 0.5..1 s.
 *   - A.3.6 equation (A.6): T_M-sequence for TYPE_0 (m = n = 2) is at most
 *     4 * 11 + 10 (t_A, A.5) + 1 (t1, A.3) + 3 (t2, A.4) = 58 T_BIT.
 *   - A.1.6 equations (A.1): checksum seed 0x52, XOR, 8 -> 6 bit compression.
 *     Worked by hand (and cross-checked with a separate script) for the bytes
 *     used here:
 *       test message [0xA2, CKT]:  0x52 ^ 0xA2 ^ 0x00 = 0xF0 -> Checksum6 0x00
 *       reply [0x1E, CKS]:         0x52 ^ 0x1E ^ 0x00 = 0x4C -> Checksum6 0x28
 *
 * Tick unit is 100 us; durations are rounded up to whole ticks.
 */
#include <setjmp.h>
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#include <cmocka.h>

#include "iolinki_master/master.h"

/* Table 46 T1: TYPE_0 read of Direct Parameter page 1 address 0x02. */
static const uint8_t k_test_message[2] = {0xA2U, 0x00U};
/* MinCycleTime 0x1E (3.0 ms, Table B.3) with the A.1.6 checksum, no flags. */
static const uint8_t k_reply[2] = {0x1EU, 0x28U};
/* Same octet with a wrong checksum: undecodable (Table 46 T4). */
static const uint8_t k_bad_reply[2] = {0x1EU, 0x00U};

/* Timings in 100 us ticks, derived from the spec values in the header. */
#define T_REN_TICKS 5U         /* 500 us, Table 10 */
#define T_DMT_COM3_TICKS 2U    /* 32 * 4.34 us = 139 us */
#define T_DMT_COM2_TICKS 9U    /* 32 * 26.04 us = 833 us */
#define T_DMT_COM1_TICKS 67U   /* 32 * 208.33 us = 6667 us */
#define T_MSEQ_COM3_TICKS 3U   /* 58 * 4.34 us = 252 us */
#define T_MSEQ_COM2_TICKS 16U  /* 58 * 26.04 us = 1510 us */
#define T_MSEQ_COM1_TICKS 121U /* 58 * 208.33 us = 12083 us */
#define T_DWU_TICKS 400U       /* 40 ms default, inside 30..50 ms */
#define T_SD_TICKS 5000U       /* 500 ms default, inside 0.5..1 s */

#define MAX_EVENTS 64

typedef enum
{
    EV_WAKE,
    EV_SEND,
} event_kind_t;

typedef struct
{
    event_kind_t kind;
    uint32_t at;
    iolink_baudrate_t baud;
    uint8_t data[4];
    size_t len;
} wire_event_t;

static wire_event_t g_events[MAX_EVENTS];
static int g_event_count;
static uint32_t g_now;
static iolink_baudrate_t g_baud;
static iolink_phy_mode_t g_mode;
static int g_inactive_calls;
/* Rate the simulated device answers at; it supports exactly one (7.3.2.2). */
static iolink_baudrate_t g_device_baud;
static bool g_device_present;
static bool g_device_corrupt_at_com3;
static uint8_t g_rx[8];
static uint8_t g_rx_len;
static uint8_t g_rx_pos;

static void record(event_kind_t kind, const uint8_t* data, size_t len)
{
    assert_in_range(g_event_count, 0, MAX_EVENTS - 1);
    g_events[g_event_count].kind = kind;
    g_events[g_event_count].at = g_now;
    g_events[g_event_count].baud = g_baud;
    g_events[g_event_count].len = len;
    if (len > 0U) {
        memcpy(g_events[g_event_count].data, data, len);
    }
    g_event_count++;
}

static void queue_rx(const uint8_t* data, uint8_t len)
{
    memcpy(g_rx, data, len);
    g_rx_len = len;
    g_rx_pos = 0U;
}

static int phy_send(void* user, const uint8_t* data, size_t len)
{
    (void) user;
    assert_in_range(len, 1U, 4U);
    record(EV_SEND, data, len);

    if (g_device_present && (len == 2U) && (memcmp(data, k_test_message, 2U) == 0)) {
        if (g_baud == g_device_baud) {
            queue_rx(k_reply, sizeof(k_reply));
        }
        else if (g_device_corrupt_at_com3 && (g_baud == IOLINK_BAUDRATE_COM3)) {
            queue_rx(k_bad_reply, sizeof(k_bad_reply));
        }
    }
    return (int) len;
}

static int phy_recv_byte(void* user, uint8_t* byte)
{
    (void) user;
    if (g_rx_pos >= g_rx_len) {
        return 0;
    }
    *byte = g_rx[g_rx_pos++];
    return 1;
}

static int hook_set_mode(iolink_phy_mode_t mode)
{
    g_mode = mode;
    if (mode == IOLINK_PHY_MODE_INACTIVE) {
        g_inactive_calls++;
    }
    return 0;
}

static int hook_set_baudrate(iolink_baudrate_t baudrate)
{
    g_baud = baudrate;
    return 0;
}

static int hook_flush_rx(void)
{
    g_rx_len = 0U;
    g_rx_pos = 0U;
    return 0;
}

static int hook_ok(void)
{
    return 0;
}

static int hook_wake_up(void)
{
    record(EV_WAKE, NULL, 0U);
    return 0;
}

static const iolink_phy_api_t g_phy = {
    .send = phy_send,
    .recv_byte = phy_recv_byte,
};

static iolink_master_config_t make_config(void)
{
    iolink_master_config_t config;

    memset(&config, 0, sizeof(config));
    config.port_mode = IOLINK_MASTER_PORT_MODE_IOLINK;
    config.m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_1_1;
    config.baudrate = IOLINK_BAUDRATE_COM3;
    config.min_cycle_time = 100U;
    config.pd_in_len = 1U;
    config.auto_baudrate = true;
    config.set_mode_checked = hook_set_mode;
    config.set_baudrate_checked = hook_set_baudrate;
    config.flush_rx = hook_flush_rx;
    config.prepare_tx = hook_ok;
    config.prepare_rx = hook_ok;
    config.wake_up = hook_wake_up;
    return config;
}

static int reset(void** state)
{
    (void) state;
    memset(g_events, 0, sizeof(g_events));
    g_event_count = 0;
    g_now = 0U;
    g_baud = IOLINK_BAUDRATE_COM1;
    g_mode = IOLINK_PHY_MODE_INACTIVE;
    g_inactive_calls = 0;
    g_device_baud = IOLINK_BAUDRATE_COM1;
    g_device_present = false;
    g_device_corrupt_at_com3 = false;
    g_rx_len = 0U;
    g_rx_pos = 0U;
    return 0;
}

/* Tick the controller (the path real firmware uses) every 100 us until
 * @p until, inclusive. */
static void run_until(iolink_master_controller_t* ctrl, uint32_t until)
{
    for (; g_now <= until; g_now++) {
        (void) iolink_master_controller_tick_at(ctrl, g_now);
    }
}

static void assert_test_message(int index, uint32_t at, iolink_baudrate_t baud)
{
    assert_int_equal(g_events[index].kind, EV_SEND);
    assert_int_equal(g_events[index].at, at);
    assert_int_equal(g_events[index].baud, baud);
    assert_int_equal(g_events[index].len, 2U);
    assert_memory_equal(g_events[index].data, k_test_message, 2U);
}

static void assert_wake(int index, uint32_t at)
{
    assert_int_equal(g_events[index].kind, EV_WAKE);
    assert_int_equal(g_events[index].at, at);
}

/*
 * 7.3.2.2 / Figure 31 / Figure 32 / Figure 36: one wake-up, then a test
 * message at COM3, COM2, COM1, each preceded by T_DMT at its own rate (the
 * first also by T_REN), each given T_M-sequence to be answered. No answer at
 * COM1 increments Retry and the next wake-up waits T_DWU.
 */
static void test_silent_port_follows_figure_36_timing(void** state)
{
    iolink_master_controller_t ctrl;
    iolink_master_port_t port;
    iolink_master_config_t config = make_config();
    uint32_t com3_at;
    uint32_t com2_at;
    uint32_t com1_at;
    uint32_t wake2_at;

    (void) state;

    assert_int_equal(iolink_master_controller_init(&ctrl, &port, 1U, &g_phy, &config), 0);
    assert_int_equal(g_baud, IOLINK_BAUDRATE_COM3);
    assert_int_equal(g_mode, IOLINK_PHY_MODE_SDCI);

    com3_at = 0U + T_REN_TICKS + T_DMT_COM3_TICKS;
    com2_at = com3_at + T_MSEQ_COM3_TICKS + T_DMT_COM2_TICKS;
    com1_at = com2_at + T_MSEQ_COM2_TICKS + T_DMT_COM1_TICKS;
    wake2_at = com1_at + T_MSEQ_COM1_TICKS + T_DWU_TICKS;

    run_until(&ctrl, wake2_at - 1U);
    assert_int_equal(g_event_count, 4);
    assert_wake(0, 0U);
    assert_test_message(1, com3_at, IOLINK_BAUDRATE_COM3);
    assert_test_message(2, com2_at, IOLINK_BAUDRATE_COM2);
    assert_test_message(3, com1_at, IOLINK_BAUDRATE_COM1);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_STARTUP);

    /* Figure 32: the repeated wake-up comes T_DWU after the failed attempt and
       the scan starts again at COM3 (Figure 36 T19, T15). */
    run_until(&ctrl, wake2_at);
    assert_int_equal(g_event_count, 5);
    assert_wake(4, wake2_at);
    assert_int_equal(g_baud, IOLINK_BAUDRATE_COM3);
    assert_true(T_DWU_TICKS >= 300U && T_DWU_TICKS <= 500U);
}

/*
 * Figure 33 / Figure 36 T5 / 7.3.2.2: n_WU + 1 = 3 wake-ups per sequence. After
 * the third silent attempt the PHY goes inactive, the failure is counted, and
 * the next sequence starts no earlier than T_SD later with the PHY back in SDCI.
 */
static void test_failed_sequence_goes_inactive_then_waits_t_sd(void** state)
{
    iolink_master_controller_t ctrl;
    iolink_master_port_t port;
    iolink_master_config_t config = make_config();
    iolink_master_diagnostics_t diagnostics;
    uint32_t attempt = T_REN_TICKS + T_DMT_COM3_TICKS + T_MSEQ_COM3_TICKS + T_DMT_COM2_TICKS +
                       T_MSEQ_COM2_TICKS + T_DMT_COM1_TICKS + T_MSEQ_COM1_TICKS;
    uint32_t failed_at = 3U * attempt + 2U * T_DWU_TICKS;
    int wakes = 0;
    int messages = 0;
    int i;

    (void) state;

    assert_int_equal(iolink_master_controller_init(&ctrl, &port, 1U, &g_phy, &config), 0);

    run_until(&ctrl, failed_at - 1U);
    assert_int_equal(g_inactive_calls, 0);

    run_until(&ctrl, failed_at);
    assert_int_equal(g_inactive_calls, 1);
    assert_int_equal(g_mode, IOLINK_PHY_MODE_INACTIVE);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_STARTUP);
    assert_int_equal(iolink_master_get_diagnostics(&port, &diagnostics), 0);
    assert_int_equal(diagnostics.establish_failures, 1U);

    for (i = 0; i < g_event_count; i++) {
        if (g_events[i].kind == EV_WAKE) {
            wakes++;
        }
        else {
            messages++;
        }
    }
    assert_int_equal(wakes, 3);    /* n_WU + 1, Table 42 */
    assert_int_equal(messages, 9); /* COM3, COM2, COM1 per wake-up */

    /* Nothing goes out during T_SD. */
    run_until(&ctrl, failed_at + T_SD_TICKS - 1U);
    assert_int_equal(g_event_count, 12);
    assert_int_equal(g_mode, IOLINK_PHY_MODE_INACTIVE);

    run_until(&ctrl, failed_at + T_SD_TICKS);
    assert_int_equal(g_event_count, 13);
    assert_wake(12, failed_at + T_SD_TICKS);
    assert_int_equal(g_mode, IOLINK_PHY_MODE_SDCI);
    assert_true(T_SD_TICKS >= 5000U && T_SD_TICKS <= 10000U);
}

/*
 * Figure 31 example: COM3 and COM2 test messages go unanswered, the device
 * answers at COM1, and the master stops the scan there (7.3.2.2, Figure 36 T4)
 * with a single wake-up.
 */
static void test_figure_31_device_answers_at_com1(void** state)
{
    iolink_master_controller_t ctrl;
    iolink_master_port_t port;
    iolink_master_config_t config = make_config();
    iolink_master_device_info_t info;
    uint32_t com1_at = T_REN_TICKS + T_DMT_COM3_TICKS + T_MSEQ_COM3_TICKS + T_DMT_COM2_TICKS +
                       T_MSEQ_COM2_TICKS + T_DMT_COM1_TICKS;

    (void) state;

    g_device_present = true;
    g_device_baud = IOLINK_BAUDRATE_COM1;
    assert_int_equal(iolink_master_controller_init(&ctrl, &port, 1U, &g_phy, &config), 0);

    run_until(&ctrl, com1_at + 1U);
    assert_wake(0, 0U);
    assert_test_message(1, T_REN_TICKS + T_DMT_COM3_TICKS, IOLINK_BAUDRATE_COM3);
    assert_test_message(3, com1_at, IOLINK_BAUDRATE_COM1);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_PREOPERATE);
    assert_int_equal(g_baud, IOLINK_BAUDRATE_COM1);
    assert_int_equal(iolink_master_get_device_info(&port, &info), IOLINK_MASTER_STATUS_PENDING);
    assert_int_equal(info.min_cycle_time, 0x1EU);

    for (int i = 0; i < g_event_count; i++) {
        if (i != 0) {
            assert_int_equal(g_events[i].kind, EV_SEND);
        }
    }
}

/*
 * Table 46 AwaitReply_1 / T4: an answer that cannot be decoded is no answer.
 * The device here only works at COM2; at COM3 the line carries a reply with a
 * wrong A.1.6 checksum. The master must not accept it and must try COM2 next.
 */
static void test_undecodable_answer_moves_scan_to_next_rate(void** state)
{
    iolink_master_controller_t ctrl;
    iolink_master_port_t port;
    iolink_master_config_t config = make_config();
    iolink_master_diagnostics_t diagnostics;
    uint32_t com2_at = T_REN_TICKS + T_DMT_COM3_TICKS + T_MSEQ_COM3_TICKS + T_DMT_COM2_TICKS;

    (void) state;

    g_device_present = true;
    g_device_baud = IOLINK_BAUDRATE_COM2;
    g_device_corrupt_at_com3 = true;
    assert_int_equal(iolink_master_controller_init(&ctrl, &port, 1U, &g_phy, &config), 0);

    run_until(&ctrl, com2_at - 1U);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_STARTUP);
    assert_int_equal(iolink_master_get_diagnostics(&port, &diagnostics), 0);
    assert_int_equal(diagnostics.checksum_errors, 1U);

    run_until(&ctrl, com2_at + 1U);
    assert_test_message(2, com2_at, IOLINK_BAUDRATE_COM2);
    assert_int_equal(iolink_master_get_state(&port), IOLINK_MASTER_STATE_PREOPERATE);
    assert_int_equal(g_baud, IOLINK_BAUDRATE_COM2);
}

/*
 * A port restricted to one rate (auto_baudrate = false) keeps the wake-up and
 * retry timing: WURQ, T_REN + T_DMT, test message, T_M-sequence, T_DWU, WURQ.
 * This is the configuration the LabWired on-wire harness uses (COM2).
 */
static void test_fixed_rate_port_spaces_wake_ups_by_t_dwu(void** state)
{
    iolink_master_controller_t ctrl;
    iolink_master_port_t port;
    iolink_master_config_t config = make_config();
    uint32_t msg_at = T_REN_TICKS + T_DMT_COM2_TICKS;
    uint32_t wake2_at = msg_at + T_MSEQ_COM2_TICKS + T_DWU_TICKS;

    (void) state;

    config.auto_baudrate = false;
    config.baudrate = IOLINK_BAUDRATE_COM2;
    assert_int_equal(iolink_master_controller_init(&ctrl, &port, 1U, &g_phy, &config), 0);

    run_until(&ctrl, wake2_at);
    assert_int_equal(g_event_count, 3);
    assert_wake(0, 0U);
    assert_test_message(1, msg_at, IOLINK_BAUDRATE_COM2);
    assert_wake(2, wake2_at);
}

/* The scheduler hint reports the end of the current wait instead of "now". */
static void test_next_tick_time_reports_startup_waits(void** state)
{
    iolink_master_port_t port;
    iolink_master_config_t config = make_config();
    uint32_t next = 0U;

    (void) state;

    assert_int_equal(iolink_master_init(&port, &g_phy, &config), 0);
    g_now = 10U;
    assert_int_equal(iolink_master_tick_at(&port, IOLINK_MASTER_TICK_CYCLE_DUE, 10U), 0);
    assert_int_equal(iolink_master_get_next_tick_time(&port, 11U, &next), 0);
    assert_int_equal(next, 10U + T_REN_TICKS + T_DMT_COM3_TICKS);
}

int main(void)
{
    const struct CMUnitTest tests[] = {
        cmocka_unit_test_setup(test_silent_port_follows_figure_36_timing, reset),
        cmocka_unit_test_setup(test_failed_sequence_goes_inactive_then_waits_t_sd, reset),
        cmocka_unit_test_setup(test_figure_31_device_answers_at_com1, reset),
        cmocka_unit_test_setup(test_undecodable_answer_moves_scan_to_next_rate, reset),
        cmocka_unit_test_setup(test_fixed_rate_port_spaces_wake_ups_by_t_dwu, reset),
        cmocka_unit_test_setup(test_next_tick_time_reports_startup_waits, reset),
    };

    return cmocka_run_group_tests(tests, NULL, NULL);
}
