#include <setjmp.h>
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#include <cmocka.h>

#include "iolinki/device.h"
#include "iolinki/events.h"
#include "iolinki/protocol.h"
#include "iolinki_master/master.h"

#define LINK_QUEUE_CAP 128U
#define MAX_PD_LEN 32U

typedef struct {
  uint8_t bytes[LINK_QUEUE_CAP];
  uint8_t head;
  uint8_t len;
} link_queue_t;

static link_queue_t g_master_to_device;
static link_queue_t g_device_to_master;
static iolink_device_ctx_t g_device_ctx;
static iolink_config_t g_device_stack_config;
static iolink_device_config_t g_device_runtime_config;
static int g_wakeup_pending;
static uint8_t g_last_pd_input[MAX_PD_LEN];
static uint8_t g_last_pd_input_len;
static uint8_t g_last_pd_output[MAX_PD_LEN];
static uint8_t g_last_pd_output_len;

static void q_reset(link_queue_t *q) { memset(q, 0, sizeof(*q)); }

static int q_push(link_queue_t *q, const uint8_t *data, size_t len) {
  size_t i;

  if ((data == NULL) || ((size_t)q->len + len > LINK_QUEUE_CAP)) {
    return -1;
  }

  for (i = 0U; i < len; i++) {
    uint8_t tail = (uint8_t)((q->head + q->len) % LINK_QUEUE_CAP);
    q->bytes[tail] = data[i];
    q->len++;
  }

  return (int)len;
}

static int q_pop(link_queue_t *q, uint8_t *byte) {
  if ((byte == NULL) || (q->len == 0U)) {
    return 0;
  }

  *byte = q->bytes[q->head];
  q->head = (uint8_t)((q->head + 1U) % LINK_QUEUE_CAP);
  q->len--;
  return 1;
}

static int master_send(void *user, const uint8_t *data, size_t len) {
  (void)user;
  return q_push(&g_master_to_device, data, len);
}

static int master_recv(void *user, uint8_t *byte) {
  (void)user;
  return q_pop(&g_device_to_master, byte);
}

static int master_wake_up(void) {
  g_wakeup_pending = 1;
  return 0;
}

static int checked_set_mode(iolink_phy_mode_t mode) {
  (void)mode;
  return 0;
}

static int phy_noop(void) { return 0; }

static int device_phy_noop(void *user) {
  (void)user;
  return 0;
}

static int device_send(void *user, const uint8_t *data, size_t len) {
  (void)user;
  return q_push(&g_device_to_master, data, len);
}

static int device_recv(void *user, uint8_t *byte) {
  (void)user;
  return q_pop(&g_master_to_device, byte);
}

static int device_detect_wakeup(void *user) {
  (void)user;
  int ret = g_wakeup_pending;
  g_wakeup_pending = 0;
  return ret;
}

static void device_set_mode(void *user, iolink_phy_mode_t mode) {
  (void)user;
  (void)mode;
}

static void device_set_baudrate(void *user, iolink_baudrate_t baudrate) {
  (void)user;
  (void)baudrate;
}

static void on_device_pd_input(const uint8_t *data, uint8_t len) {
  assert_true(len <= sizeof(g_last_pd_input));
  memcpy(g_last_pd_input, data, len);
  g_last_pd_input_len = len;
}

static void on_device_pd_output(uint8_t *data, uint8_t len) {
  assert_true(len <= sizeof(g_last_pd_output));
  memcpy(g_last_pd_output, data, len);
  g_last_pd_output_len = len;
}

static void fill_incrementing(uint8_t *data, uint8_t len, uint8_t first) {
  uint8_t i;

  for (i = 0U; i < len; i++) {
    data[i] = (uint8_t)(first + i);
  }
}

static void pump_device(const uint8_t *pd, uint8_t len) {
  uint8_t i;

  assert_int_equal(iolink_device_pd_input_update(&g_device_ctx, pd, len, true),
                   0);
  for (i = 0U; i < 4U; i++) {
    iolink_device_process(&g_device_ctx);
  }
}

static iolink_m_seq_type_t
device_mseq_for_master(iolink_master_m_seq_type_t type) {
  switch (type) {
  case IOLINK_MASTER_M_SEQ_TYPE_1_1:
    return IOLINK_M_SEQ_TYPE_1_1;
  case IOLINK_MASTER_M_SEQ_TYPE_1_2:
    return IOLINK_M_SEQ_TYPE_1_2;
  case IOLINK_MASTER_M_SEQ_TYPE_1_V:
    return IOLINK_M_SEQ_TYPE_1_V;
  case IOLINK_MASTER_M_SEQ_TYPE_2_1:
    return IOLINK_M_SEQ_TYPE_2_1;
  case IOLINK_MASTER_M_SEQ_TYPE_2_2:
    return IOLINK_M_SEQ_TYPE_2_2;
  case IOLINK_MASTER_M_SEQ_TYPE_2_V:
    return IOLINK_M_SEQ_TYPE_2_V;
  case IOLINK_MASTER_M_SEQ_TYPE_0:
  default:
    return IOLINK_M_SEQ_TYPE_0;
  }
}

static void assert_bytes_equal(const uint8_t *actual, const uint8_t *expected,
                               uint8_t len) {
  uint8_t i;

  for (i = 0U; i < len; i++) {
    assert_int_equal(actual[i], expected[i]);
  }
}

static uint8_t expected_direct_param_pd_descriptor(uint8_t octets) {
  if (octets == 0U) {
    return 0x00U;
  }
  if (octets <= 2U) {
    return (uint8_t)(octets * 8U);
  }
  return (uint8_t)(0x80U | (uint8_t)(octets - 1U));
}

static int drive_real_stack_read_isdu(iolink_master_port_t *master,
                                      uint16_t index, uint8_t subindex,
                                      uint8_t pd_in_len, uint8_t *data,
                                      uint8_t *len);

static void assert_real_stack_isdu_read(iolink_master_port_t *master,
                                        uint16_t index, uint8_t pd_in_len,
                                        const uint8_t *expected,
                                        uint8_t expected_len) {
  uint8_t data[64] = {0U};
  uint8_t len = sizeof(data);

  assert_int_equal(
      drive_real_stack_read_isdu(master, index, 0U, pd_in_len, data, &len),
      IOLINK_MASTER_STATUS_OK);
  assert_int_equal(len, expected_len);
  assert_memory_equal(data, expected, expected_len);
}

static void
assert_master_real_stack_profile(iolink_master_m_seq_type_t m_seq_type,
                                 uint8_t pd_in_len, uint8_t pd_out_len,
                                 uint8_t pd_value, const char *case_name) {
  static const iolink_phy_api_t master_phy = {
      .send = master_send,
      .recv_byte = master_recv,
  };
  static const iolink_phy_api_t device_phy = {
      .init = device_phy_noop,
      .set_mode = device_set_mode,
      .set_baudrate = device_set_baudrate,
      .send = device_send,
      .recv_byte = device_recv,
      .detect_wakeup = device_detect_wakeup,
  };
  static const iolink_app_callbacks_t app_callbacks = {
      .on_pd_input = on_device_pd_input,
      .on_pd_output = on_device_pd_output,
  };
  iolink_master_port_t master;
  iolink_master_config_t master_config = {
      .port_mode = IOLINK_MASTER_PORT_MODE_IOLINK,
      .m_seq_type = m_seq_type,
      .baudrate = IOLINK_BAUDRATE_COM2,
      .min_cycle_time = 10U,
      .pd_in_len = pd_in_len,
      .pd_out_len = pd_out_len,
      .response_timeout_100us = 20U,
      .set_mode_checked = checked_set_mode,
      .prepare_tx = phy_noop,
      .prepare_rx = phy_noop,
      .wake_up = master_wake_up,
  };
  g_device_stack_config = (iolink_config_t){
      .m_seq_type = device_mseq_for_master(m_seq_type),
      .min_cycle_time = 10U,
      .pd_in_len = pd_in_len,
      .pd_out_len = pd_out_len,
      .t_pd_us = 0U,
  };
  g_device_runtime_config = (iolink_device_config_t){
      .phy = device_phy,
      .stack = g_device_stack_config,
      .app_callbacks = &app_callbacks,
  };
  uint8_t pd_in[MAX_PD_LEN] = {0U};
  uint8_t pd_out[MAX_PD_LEN] = {0U};
  uint8_t pd_len = 0U;
  uint8_t device_pd[MAX_PD_LEN] = {0U};
  uint8_t i;

  q_reset(&g_master_to_device);
  q_reset(&g_device_to_master);
  g_wakeup_pending = 0;
  g_last_pd_input_len = 0U;
  g_last_pd_output_len = 0U;
  memset(g_last_pd_input, 0, sizeof(g_last_pd_input));
  memset(g_last_pd_output, 0, sizeof(g_last_pd_output));
  fill_incrementing(device_pd, pd_in_len, pd_value);
  fill_incrementing(pd_out, pd_out_len, (uint8_t)(pd_value ^ 0x55U));

  assert_int_equal(iolink_device_init(&g_device_ctx, &g_device_runtime_config),
                   0);
  iolink_device_set_timing_enforcement(&g_device_ctx, false);
  assert_int_equal(iolink_master_init(&master, &master_phy, &master_config), 0);
  assert_int_equal(iolink_master_set_pd_out(&master, pd_out, pd_out_len), 0);

  for (i = 0U; i < 20U; i++) {
    assert_int_equal(
        iolink_master_tick_at(&master, IOLINK_MASTER_TICK_CYCLE_DUE, i), 0);
    pump_device(device_pd, pd_in_len);
    (void)iolink_master_tick_at(&master, IOLINK_MASTER_TICK_NONE, i + 1U);

    if (iolink_master_get_state(&master) == IOLINK_MASTER_STATE_OPERATE) {
      assert_int_equal(
          iolink_master_tick_at(&master, IOLINK_MASTER_TICK_CYCLE_DUE, i + 40U),
          0);
      pump_device(device_pd, pd_in_len);
      assert_int_equal(
          iolink_master_tick_at(&master, IOLINK_MASTER_TICK_NONE, i + 41U), 1);
      assert_int_equal(
          iolink_master_get_pd_in(&master, pd_in, sizeof(pd_in), &pd_len), 0);
      assert_int_equal(pd_len, pd_in_len);
      assert_bytes_equal(pd_in, device_pd, pd_in_len);
      assert_int_equal(g_last_pd_input_len, pd_in_len);
      assert_bytes_equal(g_last_pd_input, device_pd, pd_in_len);
      assert_int_equal(g_last_pd_output_len, pd_out_len);
      assert_bytes_equal(g_last_pd_output, pd_out, pd_out_len);
      return;
    }
  }

  fail_msg("real iolinki device stack did not reach OPERATE for %s", case_name);
}

static void init_master_and_real_device_in_operate(
    iolink_master_port_t *master, iolink_master_m_seq_type_t m_seq_type,
    uint8_t pd_in_len, uint8_t pd_out_len, const uint8_t *pd_out) {
  static const iolink_phy_api_t master_phy = {
      .send = master_send,
      .recv_byte = master_recv,
  };
  static const iolink_phy_api_t device_phy = {
      .init = device_phy_noop,
      .set_mode = device_set_mode,
      .set_baudrate = device_set_baudrate,
      .send = device_send,
      .recv_byte = device_recv,
      .detect_wakeup = device_detect_wakeup,
  };
  static const iolink_app_callbacks_t app_callbacks = {
      .on_pd_input = on_device_pd_input,
      .on_pd_output = on_device_pd_output,
  };
  iolink_master_config_t master_config = {
      .port_mode = IOLINK_MASTER_PORT_MODE_IOLINK,
      .m_seq_type = m_seq_type,
      .baudrate = IOLINK_BAUDRATE_COM2,
      .min_cycle_time = 10U,
      .pd_in_len = pd_in_len,
      .pd_out_len = pd_out_len,
      .response_timeout_100us = 20U,
      .set_mode_checked = checked_set_mode,
      .prepare_tx = phy_noop,
      .prepare_rx = phy_noop,
      .wake_up = master_wake_up,
  };
  g_device_stack_config = (iolink_config_t){
      .m_seq_type = device_mseq_for_master(m_seq_type),
      .min_cycle_time = 10U,
      .pd_in_len = pd_in_len,
      .pd_out_len = pd_out_len,
      .t_pd_us = 0U,
  };
  g_device_runtime_config = (iolink_device_config_t){
      .phy = device_phy,
      .stack = g_device_stack_config,
      .app_callbacks = &app_callbacks,
  };
  uint8_t device_pd[MAX_PD_LEN] = {0U};
  uint8_t i;

  q_reset(&g_master_to_device);
  q_reset(&g_device_to_master);
  g_wakeup_pending = 0;
  g_last_pd_input_len = 0U;
  g_last_pd_output_len = 0U;
  memset(g_last_pd_input, 0, sizeof(g_last_pd_input));
  memset(g_last_pd_output, 0, sizeof(g_last_pd_output));
  fill_incrementing(device_pd, pd_in_len, 0xA0U);

  assert_int_equal(iolink_device_init(&g_device_ctx, &g_device_runtime_config),
                   0);
  iolink_device_set_timing_enforcement(&g_device_ctx, false);
  assert_int_equal(iolink_master_init(master, &master_phy, &master_config), 0);
  assert_int_equal(iolink_master_set_pd_out(master, pd_out, pd_out_len), 0);

  for (i = 0U; i < 20U; i++) {
    assert_int_equal(
        iolink_master_tick_at(master, IOLINK_MASTER_TICK_CYCLE_DUE, i), 0);
    pump_device(device_pd, pd_in_len);
    (void)iolink_master_tick_at(master, IOLINK_MASTER_TICK_NONE, i + 1U);
    if (iolink_master_get_state(master) == IOLINK_MASTER_STATE_OPERATE) {
      return;
    }
  }

  fail_msg("real iolinki device stack did not reach OPERATE for ISDU test");
}

static int drive_real_stack_read_isdu(iolink_master_port_t *master,
                                      uint16_t index, uint8_t subindex,
                                      uint8_t pd_in_len, uint8_t *data,
                                      uint8_t *len) {
  uint8_t device_pd[MAX_PD_LEN] = {0U};
  uint8_t cycle;
  int ret;

  fill_incrementing(device_pd, pd_in_len, 0xC0U);
  ret = iolink_master_read_isdu(master, index, subindex, data, len);
  if (ret != IOLINK_MASTER_STATUS_PENDING) {
    return ret;
  }

  for (cycle = 0U; cycle < 80U; cycle++) {
    assert_int_equal(iolink_master_tick_at(master, IOLINK_MASTER_TICK_CYCLE_DUE,
                                           (uint32_t)(100U + (cycle * 12U))),
                     0);
    pump_device(device_pd, pd_in_len);
    (void)iolink_master_tick_at(master, IOLINK_MASTER_TICK_NONE,
                                (uint32_t)(101U + (cycle * 12U)));

    ret = iolink_master_read_isdu(master, index, subindex, data, len);
    if (ret != IOLINK_MASTER_STATUS_PENDING) {
      return ret;
    }
  }

  return IOLINK_MASTER_STATUS_PENDING;
}

static void drive_real_stack_cycle(iolink_master_port_t *master,
                                   uint8_t pd_in_len, uint32_t now_100us);

static int drive_real_stack_write_isdu(iolink_master_port_t *master,
                                       uint16_t index, uint8_t subindex,
                                       uint8_t pd_in_len, const uint8_t *data,
                                       uint8_t len) {
  uint8_t cycle;
  int ret;

  ret = iolink_master_write_isdu(master, index, subindex, data, len);
  if (ret != IOLINK_MASTER_STATUS_PENDING) {
    return ret;
  }

  for (cycle = 0U; cycle < 80U; cycle++) {
    drive_real_stack_cycle(master, pd_in_len, (uint32_t)(700U + (cycle * 12U)));
    ret = iolink_master_write_isdu(master, index, subindex, data, len);
    if (ret != IOLINK_MASTER_STATUS_PENDING) {
      return ret;
    }
  }

  return IOLINK_MASTER_STATUS_PENDING;
}

static int drive_real_stack_parameter_block(iolink_master_port_t *master,
                                            uint16_t index, uint8_t subindex,
                                            uint8_t pd_in_len,
                                            const uint8_t *data, uint8_t len) {
  uint8_t cycle;
  int ret;

  ret = iolink_master_write_parameter_block(master, index, subindex, data, len);
  if (ret != IOLINK_MASTER_STATUS_PENDING) {
    return ret;
  }

  for (cycle = 0U; cycle < 120U; cycle++) {
    drive_real_stack_cycle(master, pd_in_len, (uint32_t)(900U + (cycle * 12U)));
    ret =
        iolink_master_write_parameter_block(master, index, subindex, data, len);
    if (ret != IOLINK_MASTER_STATUS_PENDING) {
      return ret;
    }
  }

  return IOLINK_MASTER_STATUS_PENDING;
}

static void drive_real_stack_cycle(iolink_master_port_t *master,
                                   uint8_t pd_in_len, uint32_t now_100us) {
  uint8_t device_pd[MAX_PD_LEN] = {0U};

  fill_incrementing(device_pd, pd_in_len, 0xD0U);
  assert_int_equal(
      iolink_master_tick_at(master, IOLINK_MASTER_TICK_CYCLE_DUE, now_100us),
      0);
  pump_device(device_pd, pd_in_len);
  (void)iolink_master_tick_at(master, IOLINK_MASTER_TICK_NONE, now_100us + 1U);
}

static int drive_real_stack_read_event_details(iolink_master_port_t *master,
                                               uint8_t pd_in_len,
                                               iolink_master_event_t *events,
                                               uint8_t max_events,
                                               uint8_t *out_count) {
  uint8_t cycle;
  int ret;

  ret = iolink_master_read_event_details(master, events, max_events, out_count);
  if (ret != IOLINK_MASTER_STATUS_PENDING) {
    return ret;
  }

  for (cycle = 0U; cycle < 80U; cycle++) {
    drive_real_stack_cycle(master, pd_in_len, (uint32_t)(300U + (cycle * 12U)));
    ret =
        iolink_master_read_event_details(master, events, max_events, out_count);
    if (ret != IOLINK_MASTER_STATUS_PENDING) {
      return ret;
    }
  }

  return IOLINK_MASTER_STATUS_PENDING;
}

static int drive_real_stack_ack_event(iolink_master_port_t *master,
                                      uint8_t pd_in_len, uint16_t *event_code) {
  uint8_t cycle;
  int ret;

  ret = iolink_master_ack_event(master, event_code);
  if (ret != IOLINK_MASTER_STATUS_PENDING) {
    return ret;
  }

  for (cycle = 0U; cycle < 80U; cycle++) {
    drive_real_stack_cycle(master, pd_in_len, (uint32_t)(500U + (cycle * 12U)));
    ret = iolink_master_ack_event(master, event_code);
    if (ret != IOLINK_MASTER_STATUS_PENDING) {
      return ret;
    }
  }

  return IOLINK_MASTER_STATUS_PENDING;
}

static void
test_master_conformance_matrix_with_real_iolinki_device_stack(void **state) {
  static const struct {
    iolink_master_m_seq_type_t m_seq_type;
    uint8_t pd_in_len;
    uint8_t pd_out_len;
    uint8_t pd_value;
  } cases[] = {
      {IOLINK_MASTER_M_SEQ_TYPE_1_1, 1U, 0U, 0x11U},
      {IOLINK_MASTER_M_SEQ_TYPE_1_2, 2U, 1U, 0x22U},
      {IOLINK_MASTER_M_SEQ_TYPE_2_1, 2U, 2U, 0x33U},
      {IOLINK_MASTER_M_SEQ_TYPE_2_2, 3U, 2U, 0x44U},
      {IOLINK_MASTER_M_SEQ_TYPE_1_V, 4U, 1U, 0x55U},
      {IOLINK_MASTER_M_SEQ_TYPE_2_V, 4U, 3U, 0x66U},
  };
  size_t i;

  (void)state;

  for (i = 0U; i < (sizeof(cases) / sizeof(cases[0])); i++) {
    print_message("real-stack profile case %zu\n", i);
    assert_master_real_stack_profile(cases[i].m_seq_type, cases[i].pd_in_len,
                                     cases[i].pd_out_len, cases[i].pd_value,
                                     "profile matrix");
  }
}

static void test_master_reads_direct_parameters_with_real_iolinki_device_stack(
    void **state) {
  iolink_master_port_t master;
  uint8_t pd_out[2] = {0x5AU, 0xA5U};
  uint8_t page[16] = {0U};
  uint8_t len = sizeof(page);

  (void)state;

  init_master_and_real_device_in_operate(&master, IOLINK_MASTER_M_SEQ_TYPE_2_2,
                                         3U, sizeof(pd_out), pd_out);

  assert_int_equal(drive_real_stack_read_isdu(&master,
                                              IOLINK_IDX_DIRECT_PARAMETERS_1,
                                              0U, 3U, page, &len),
                   IOLINK_MASTER_STATUS_OK);
  assert_int_equal(len, sizeof(page));
  assert_int_equal(page[0x02], 10U);
  assert_int_equal(page[0x03], 0x01U);
  assert_int_equal(page[0x04], 0x11U);
  assert_int_equal(page[0x05], expected_direct_param_pd_descriptor(3U));
  assert_int_equal(page[0x06],
                   expected_direct_param_pd_descriptor(sizeof(pd_out)));

  assert_int_equal(iolink_master_read_device_info(&master),
                   IOLINK_MASTER_STATUS_PENDING);
  len = sizeof(page);
  assert_int_equal(drive_real_stack_read_isdu(&master,
                                              IOLINK_IDX_DIRECT_PARAMETERS_1,
                                              0U, 3U, page, &len),
                   IOLINK_MASTER_STATUS_OK);
  assert_int_equal(
      iolink_master_apply_direct_parameter_page1(&master, page, len),
      IOLINK_MASTER_STATUS_OK);
  assert_int_equal(iolink_master_validate_device_info(&master),
                   IOLINK_MASTER_STATUS_OK);
}

static void
test_master_reads_mandatory_identity_objects_with_real_iolinki_device_stack(
    void **state) {
  iolink_master_port_t master;
  uint8_t pd_out[2] = {0x31U, 0x32U};
  const uint8_t vendor_id[] = {0xFFU, 0xFFU};
  const uint8_t device_id[] = {0x00U, 0x00U, 0x00U, 0x01U};
  const uint8_t profile_characteristic[] = {0x00U, 0x00U};
  const uint8_t revision_id[] = {0x00U, 0x01U};
  const uint8_t min_cycle_time[] = {10U};
  const uint8_t pdin_descriptor[] = {3U};
  static const uint8_t vendor_name[] = "iolinki";
  static const uint8_t product_name[] = "Generic IO-Link Device";
  static const uint8_t product_id[] = "IOLINK-DEV-001";
  static const uint8_t hardware_revision[] = "1.0";
  static const uint8_t firmware_revision[] = "0.1.0";

  (void)state;

  init_master_and_real_device_in_operate(&master, IOLINK_MASTER_M_SEQ_TYPE_2_2,
                                         3U, sizeof(pd_out), pd_out);

  assert_real_stack_isdu_read(&master, IOLINK_IDX_VENDOR_ID, 3U, vendor_id,
                              sizeof(vendor_id));
  assert_real_stack_isdu_read(&master, IOLINK_IDX_DEVICE_ID, 3U, device_id,
                              sizeof(device_id));
  assert_real_stack_isdu_read(&master, IOLINK_IDX_PROFILE_CHARACTERISTIC, 3U,
                              profile_characteristic,
                              sizeof(profile_characteristic));
  assert_real_stack_isdu_read(&master, IOLINK_IDX_REVISION_ID, 3U, revision_id,
                              sizeof(revision_id));
  assert_real_stack_isdu_read(&master, IOLINK_IDX_MIN_CYCLE_TIME, 3U,
                              min_cycle_time, sizeof(min_cycle_time));
  assert_real_stack_isdu_read(&master, IOLINK_IDX_PDIN_DESCRIPTOR, 3U,
                              pdin_descriptor, sizeof(pdin_descriptor));
  assert_real_stack_isdu_read(&master, IOLINK_IDX_VENDOR_NAME, 3U, vendor_name,
                              (uint8_t)(sizeof(vendor_name) - 1U));
  assert_real_stack_isdu_read(&master, IOLINK_IDX_PRODUCT_NAME, 3U,
                              product_name,
                              (uint8_t)(sizeof(product_name) - 1U));
  assert_real_stack_isdu_read(&master, IOLINK_IDX_PRODUCT_ID, 3U, product_id,
                              (uint8_t)(sizeof(product_id) - 1U));
  assert_real_stack_isdu_read(&master, IOLINK_IDX_HARDWARE_REVISION, 3U,
                              hardware_revision,
                              (uint8_t)(sizeof(hardware_revision) - 1U));
  assert_real_stack_isdu_read(&master, IOLINK_IDX_FIRMWARE_REVISION, 3U,
                              firmware_revision,
                              (uint8_t)(sizeof(firmware_revision) - 1U));
}

static void
test_master_writes_mandatory_tag_objects_with_real_iolinki_device_stack(
    void **state) {
  iolink_master_port_t master;
  uint8_t pd_out[1] = {0x6DU};
  static const uint8_t application_tag[] = "AppTagCI";
  static const uint8_t function_tag[] = "FuncTagCI";
  static const uint8_t location_tag[] = "LocTagCI";

  (void)state;

  init_master_and_real_device_in_operate(&master, IOLINK_MASTER_M_SEQ_TYPE_1_2,
                                         2U, sizeof(pd_out), pd_out);

  assert_int_equal(
      drive_real_stack_write_isdu(&master, IOLINK_IDX_APPLICATION_TAG, 0U, 2U,
                                  application_tag,
                                  (uint8_t)(sizeof(application_tag) - 1U)),
      IOLINK_MASTER_STATUS_OK);
  assert_real_stack_isdu_read(&master, IOLINK_IDX_APPLICATION_TAG, 2U,
                              application_tag,
                              (uint8_t)(sizeof(application_tag) - 1U));

  assert_int_equal(drive_real_stack_write_isdu(
                       &master, IOLINK_IDX_FUNCTION_TAG, 0U, 2U, function_tag,
                       (uint8_t)(sizeof(function_tag) - 1U)),
                   IOLINK_MASTER_STATUS_OK);
  assert_real_stack_isdu_read(&master, IOLINK_IDX_FUNCTION_TAG, 2U,
                              function_tag,
                              (uint8_t)(sizeof(function_tag) - 1U));

  assert_int_equal(drive_real_stack_write_isdu(
                       &master, IOLINK_IDX_LOCATION_TAG, 0U, 2U, location_tag,
                       (uint8_t)(sizeof(location_tag) - 1U)),
                   IOLINK_MASTER_STATUS_OK);
  assert_real_stack_isdu_read(&master, IOLINK_IDX_LOCATION_TAG, 2U,
                              location_tag,
                              (uint8_t)(sizeof(location_tag) - 1U));
}

static void test_master_reads_diagnostic_objects_with_real_iolinki_device_stack(
    void **state) {
  iolink_master_port_t master;
  uint8_t pd_out[1] = {0x7DU};
  const uint8_t ok_status[] = {0x00U};
  const uint8_t zero_error_stats[16] = {0U};

  (void)state;

  init_master_and_real_device_in_operate(&master, IOLINK_MASTER_M_SEQ_TYPE_1_2,
                                         2U, sizeof(pd_out), pd_out);

  assert_real_stack_isdu_read(&master, IOLINK_IDX_DEVICE_STATUS, 2U, ok_status,
                              sizeof(ok_status));
  assert_real_stack_isdu_read(&master, IOLINK_IDX_ERROR_STATS, 2U,
                              zero_error_stats, sizeof(zero_error_stats));
}

static void
test_master_reads_and_acks_events_with_real_iolinki_device_stack(void **state) {
  iolink_master_port_t master;
  uint8_t pd_out[1] = {0x7EU};
  iolink_master_diagnostics_t diagnostics;
  iolink_master_event_t events[2];
  uint8_t raw_details[8] = {0U};
  uint8_t raw_len = sizeof(raw_details);
  uint8_t count = 0U;
  uint16_t event_code = 0U;

  (void)state;

  init_master_and_real_device_in_operate(&master, IOLINK_MASTER_M_SEQ_TYPE_1_2,
                                         2U, sizeof(pd_out), pd_out);

  iolink_event_trigger(iolink_device_get_events_ctx(&g_device_ctx),
                       IOLINK_EVENT_COMM_TIMING, IOLINK_EVENT_TYPE_WARNING);
  drive_real_stack_cycle(&master, 2U, 200U);
  assert_int_equal(iolink_master_get_diagnostics(&master, &diagnostics), 0);
  assert_true(diagnostics.event_pending);

  assert_int_equal(drive_real_stack_read_isdu(&master,
                                              IOLINK_IDX_DETAILED_DEVICE_STATUS,
                                              0U, 2U, raw_details, &raw_len),
                   IOLINK_MASTER_STATUS_OK);
  assert_int_equal(raw_len, 3U);

  assert_int_equal(drive_real_stack_read_event_details(
                       &master, 2U, events,
                       (uint8_t)(sizeof(events) / sizeof(events[0])), &count),
                   IOLINK_MASTER_STATUS_OK);
  assert_int_equal(count, 1U);
  assert_int_equal(events[0].type, IOLINK_MASTER_EVENT_TYPE_WARNING);
  assert_int_equal(events[0].code, IOLINK_EVENT_COMM_TIMING);

  assert_int_equal(drive_real_stack_ack_event(&master, 2U, &event_code),
                   IOLINK_MASTER_STATUS_OK);
  assert_int_equal(event_code, IOLINK_EVENT_COMM_TIMING);
}

static void
test_master_writes_and_reads_data_storage_with_real_iolinki_device_stack(
    void **state) {
  iolink_master_port_t master;
  static const uint8_t ds_image[] = {
      0x00U, 0x18U, 0x00U, 0x07U, 'L', 'a', 'b', 'C', 'I', '0', '1',
  };
  uint8_t pd_out[1] = {0x21U};
  uint8_t readback[64] = {0U};
  uint8_t len = sizeof(readback);

  (void)state;

  init_master_and_real_device_in_operate(&master, IOLINK_MASTER_M_SEQ_TYPE_1_2,
                                         2U, sizeof(pd_out), pd_out);

  assert_int_equal(drive_real_stack_write_isdu(&master, IOLINK_IDX_DATA_STORAGE,
                                               0U, 2U, ds_image,
                                               sizeof(ds_image)),
                   IOLINK_MASTER_STATUS_OK);
  assert_int_equal(drive_real_stack_read_isdu(&master, IOLINK_IDX_DATA_STORAGE,
                                              0U, 2U, readback, &len),
                   IOLINK_MASTER_STATUS_OK);

  assert_true(len >= sizeof(ds_image));
  assert_memory_equal(readback, ds_image, sizeof(ds_image));
}

static void
test_master_restores_data_storage_with_real_parameter_block(void **state) {
  iolink_master_port_t master;
  static const uint8_t ds_image[] = {
      0x00U, 0x18U, 0x00U, 0x08U, 'B', 'l', 'o', 'c', 'k', 'C', 'I', '1',
  };
  uint8_t pd_out[1] = {0x42U};
  uint8_t readback[64] = {0U};
  uint8_t len = sizeof(readback);

  (void)state;

  init_master_and_real_device_in_operate(&master, IOLINK_MASTER_M_SEQ_TYPE_1_2,
                                         2U, sizeof(pd_out), pd_out);

  assert_int_equal(
      drive_real_stack_parameter_block(&master, IOLINK_IDX_DATA_STORAGE, 0U, 2U,
                                       ds_image, sizeof(ds_image)),
      IOLINK_MASTER_STATUS_OK);
  assert_int_equal(drive_real_stack_read_isdu(&master, IOLINK_IDX_DATA_STORAGE,
                                              0U, 2U, readback, &len),
                   IOLINK_MASTER_STATUS_OK);
  assert_true(len >= sizeof(ds_image));
  assert_memory_equal(readback, ds_image, sizeof(ds_image));
}

static void test_master_observes_real_device_access_locks(void **state) {
  iolink_master_port_t master;
  static const uint8_t ds_image[] = {
      0x00U, 0x18U, 0x00U, 0x08U, 'L', 'o', 'c', 'k', 'e', 'd', '0', '1',
  };
  const uint8_t lock_ds[] = {0x00U, IOLINK_LOCK_DS};
  const uint8_t unlock_all[] = {0x00U, 0x00U};
  uint8_t pd_out[1] = {0x5CU};
  uint8_t locks[2] = {0U};
  uint8_t len = sizeof(locks);
  iolink_master_diagnostics_t diagnostics;

  (void)state;

  init_master_and_real_device_in_operate(&master, IOLINK_MASTER_M_SEQ_TYPE_1_2,
                                         2U, sizeof(pd_out), pd_out);

  assert_int_equal(
      drive_real_stack_write_isdu(&master, IOLINK_IDX_DEVICE_ACCESS_LOCKS, 0U,
                                  2U, lock_ds, sizeof(lock_ds)),
      IOLINK_MASTER_STATUS_OK);
  assert_int_equal(drive_real_stack_read_isdu(&master,
                                              IOLINK_IDX_DEVICE_ACCESS_LOCKS,
                                              0U, 2U, locks, &len),
                   IOLINK_MASTER_STATUS_OK);
  assert_int_equal(len, sizeof(locks));
  assert_memory_equal(locks, lock_ds, sizeof(lock_ds));

  assert_int_equal(
      drive_real_stack_parameter_block(&master, IOLINK_IDX_DATA_STORAGE, 0U, 2U,
                                       ds_image, sizeof(ds_image)),
      IOLINK_MASTER_ISDU_ERR_DEVICE);
  assert_int_equal(iolink_master_get_diagnostics(&master, &diagnostics), 0);
  assert_int_equal(diagnostics.last_isdu_error,
                   IOLINK_ISDU_ERROR_WRITE_PROTECTED);
  assert_int_equal(diagnostics.last_service_result,
                   IOLINK_MASTER_ISDU_ERR_DEVICE);

  assert_int_equal(
      drive_real_stack_write_isdu(&master, IOLINK_IDX_DEVICE_ACCESS_LOCKS, 0U,
                                  2U, unlock_all, sizeof(unlock_all)),
      IOLINK_MASTER_STATUS_OK);
}

int main(void) {
  const struct CMUnitTest tests[] = {
      cmocka_unit_test(
          test_master_conformance_matrix_with_real_iolinki_device_stack),
      cmocka_unit_test(
          test_master_reads_direct_parameters_with_real_iolinki_device_stack),
      cmocka_unit_test(
          test_master_reads_mandatory_identity_objects_with_real_iolinki_device_stack),
      cmocka_unit_test(
          test_master_writes_mandatory_tag_objects_with_real_iolinki_device_stack),
      cmocka_unit_test(
          test_master_reads_diagnostic_objects_with_real_iolinki_device_stack),
      cmocka_unit_test(
          test_master_reads_and_acks_events_with_real_iolinki_device_stack),
      cmocka_unit_test(
          test_master_writes_and_reads_data_storage_with_real_iolinki_device_stack),
      cmocka_unit_test(
          test_master_restores_data_storage_with_real_parameter_block),
      cmocka_unit_test(test_master_observes_real_device_access_locks),
  };

  return cmocka_run_group_tests(tests, NULL, NULL);
}
