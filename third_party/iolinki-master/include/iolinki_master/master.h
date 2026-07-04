#ifndef IOLINKI_MASTER_MASTER_H
#define IOLINKI_MASTER_MASTER_H

#include <stdbool.h>
#include <stdint.h>
#include <stddef.h>

#include "iolinki/config.h"
#include "iolinki/phy.h"

typedef enum
{
    IOLINK_MASTER_M_SEQ_TYPE_0 = 0U,
    IOLINK_MASTER_M_SEQ_TYPE_1_1 = 1U,
    IOLINK_MASTER_M_SEQ_TYPE_1_2 = 2U,
    IOLINK_MASTER_M_SEQ_TYPE_1_V = 3U,
    IOLINK_MASTER_M_SEQ_TYPE_2_1 = 4U,
    IOLINK_MASTER_M_SEQ_TYPE_2_2 = 5U,
    IOLINK_MASTER_M_SEQ_TYPE_2_V = 6U,
} iolink_master_m_seq_type_t;

typedef enum
{
    IOLINK_MASTER_STATE_INACTIVE = 0,
    IOLINK_MASTER_STATE_STARTUP = 1,
    IOLINK_MASTER_STATE_PREOPERATE = 2,
    IOLINK_MASTER_STATE_OPERATE = 3,
    IOLINK_MASTER_STATE_ERROR = 4
} iolink_master_state_t;

typedef enum
{
    IOLINK_MASTER_PORT_MODE_IOLINK = 0,
    IOLINK_MASTER_PORT_MODE_DI = 1,
    IOLINK_MASTER_PORT_MODE_DQ = 2,
    IOLINK_MASTER_PORT_MODE_DEACTIVATED = 3
} iolink_master_port_mode_t;

typedef enum
{
    IOLINK_MASTER_ISDU_OP_NONE = 0,
    IOLINK_MASTER_ISDU_OP_READ = 1,
    IOLINK_MASTER_ISDU_OP_WRITE = 2
} iolink_master_isdu_op_t;

typedef enum
{
    IOLINK_MASTER_EVENT_TYPE_UNKNOWN = 0U,
    IOLINK_MASTER_EVENT_TYPE_NOTIFICATION = 1U,
    IOLINK_MASTER_EVENT_TYPE_WARNING = 2U,
    IOLINK_MASTER_EVENT_TYPE_ERROR = 3U
} iolink_master_event_type_t;

typedef enum
{
    IOLINK_MASTER_TICK_NONE = 0,
    IOLINK_MASTER_TICK_CYCLE_DUE = 1,
    IOLINK_MASTER_TICK_RESPONSE_TIMEOUT = 2
} iolink_master_tick_event_t;

typedef enum
{
    IOLINK_MASTER_STATUS_OK = 0,
    IOLINK_MASTER_STATUS_PENDING = 1,
    IOLINK_MASTER_ERR_INVALID_ARG = -1,
    IOLINK_MASTER_ERR_RETRY_LIMIT = -2,
    IOLINK_MASTER_ERR_FRAME = -2,
    IOLINK_MASTER_ERR_BUFFER_TOO_SMALL = -2,
    IOLINK_MASTER_ERR_CHECKSUM = -3,
    IOLINK_MASTER_ERR_SERVICE = -4,
    IOLINK_MASTER_ERR_INVALID_STATE = -5,
    IOLINK_MASTER_ERR_UNSUPPORTED_PHY = -6
} iolink_master_result_t;

typedef enum
{
    IOLINK_MASTER_PARAM_ERR_TOO_SHORT = -2,
    IOLINK_MASTER_PARAM_ERR_REVISION = -2,
    IOLINK_MASTER_PARAM_ERR_CYCLE_TIME = -3,
    IOLINK_MASTER_PARAM_ERR_PD_SIZE = -4,
    IOLINK_MASTER_PARAM_ERR_M_SEQUENCE = -5,
    IOLINK_MASTER_PARAM_ERR_VENDOR_ID = -6,
    IOLINK_MASTER_PARAM_ERR_DEVICE_ID = -7
} iolink_master_parameter_result_t;

/*
 * Device-identity inspection level, per the IO-Link port configuration model.
 * NO_CHECK establishes communication without comparing the device identity.
 * TYPE_COMP requires the connected device's VendorID and DeviceID to match the
 * configured expected values (a type-compatible device). IDENTICAL additionally
 * requires the device SerialNumber to match; the SerialNumber leg is not yet
 * wired here (it lives in ISDU index 0x0015, not Direct Parameter Page 1), so
 * IDENTICAL currently enforces the same VendorID/DeviceID check as TYPE_COMP.
 */
typedef enum
{
    IOLINK_MASTER_INSPECTION_NO_CHECK = 0,
    IOLINK_MASTER_INSPECTION_TYPE_COMP = 1,
    IOLINK_MASTER_INSPECTION_IDENTICAL = 2
} iolink_master_inspection_level_t;

typedef enum
{
    IOLINK_MASTER_ISDU_ERR_BUFFER_TOO_SMALL = -2,
    IOLINK_MASTER_ISDU_ERR_BUSY = -3,
    IOLINK_MASTER_ISDU_ERR_DEVICE = -4,
    IOLINK_MASTER_ISDU_ERR_INVALID_STATE = -5,
    IOLINK_MASTER_ISDU_ERR_VERIFY_FAILED = -6
} iolink_master_isdu_result_t;

typedef enum
{
    IOLINK_MASTER_SIO_ERR_WRONG_MODE = -2,
    IOLINK_MASTER_SIO_ERR_UNSUPPORTED_PHY = -3
} iolink_master_sio_result_t;

/*
 * Master Command communication channel, per the IO-Link Master Command octet
 * layout (`IOLINK_MC_COMM_CHANNEL_MASK`). The channel selects which logical
 * page/diagnosis/ISDU register a Master Command addresses.
 */
typedef enum
{
    IOLINK_MASTER_MC_CHANNEL_PROCESS = 0U,
    IOLINK_MASTER_MC_CHANNEL_PAGE = 1U,
    IOLINK_MASTER_MC_CHANNEL_DIAGNOSIS = 2U,
    IOLINK_MASTER_MC_CHANNEL_ISDU = 3U
} iolink_master_mc_channel_t;

/* Address of the MasterCommand transition register (operate transition = 0x0F). */
#define IOLINK_MASTER_MC_TRANSITION_ADDR 0x0FU

typedef struct
{
    uint8_t qualifier;
    iolink_master_event_type_t type;
    uint16_t code;
} iolink_master_event_t;

/*
 * Optional event-dispatch callbacks. When set, they turn event handling from a
 * poll-only model (read `diagnostics.event_pending` yourself) into a dispatch:
 * `event_pending_handler` fires on the rising edge of the OD Event flag during a
 * cyclic response, prompting the application to read event details;
 * `event_handler` fires once per decoded event from
 * `iolink_master_read_event_details`. Both may be NULL to keep poll-only
 * behavior. `event_user` is passed through unchanged.
 */
typedef void (*iolink_master_event_pending_cb_t)(void* user);
typedef void (*iolink_master_event_cb_t)(void* user, const iolink_master_event_t* event);

typedef struct
{
    iolink_master_port_mode_t port_mode;
    iolink_master_m_seq_type_t m_seq_type;
    iolink_baudrate_t baudrate;
    uint8_t min_cycle_time;
    uint8_t pd_in_len;
    uint8_t pd_out_len;
    bool auto_baudrate;
    bool validate_device_info;
    iolink_master_inspection_level_t inspection_level;
    uint16_t expected_vendor_id;
    uint32_t expected_device_id;
    uint8_t response_timeout_100us;
    /*
     * Number of extra wake-up requests to issue at the current baudrate before
     * giving up (auto-baud: advancing to the next COM rate; fixed baud: erroring).
     * 0 preserves the historical "one attempt then advance/error" behavior; real
     * hardware bring-up should set this to a small count (the spec allows the
     * master to retry the wake-up sequence) so a device that misses the first
     * WURQ still links up.
     */
    uint8_t wake_retry_limit;
    void* event_user;
    iolink_master_event_pending_cb_t event_pending_handler;
    iolink_master_event_cb_t event_handler;
    int (*set_mode_checked)(iolink_phy_mode_t mode);
    int (*set_baudrate_checked)(iolink_baudrate_t baudrate);
    int (*read_cq_line_checked)(void);
    int (*flush_rx)(void);
    int (*prepare_tx)(void);
    int (*prepare_rx)(void);
    int (*read_cq_line)(void);
    int (*wake_up)(void);
} iolink_master_config_t;

typedef struct
{
    uint8_t od_status;
    bool event_pending;
    uint8_t rx_retry_count;
    uint32_t checksum_errors;
    uint32_t send_errors;
    uint32_t response_timeouts;
    uint32_t cycle_slips;
    uint32_t last_cycle_jitter_100us;
    uint32_t max_cycle_jitter_100us;
    int supply_voltage_mv;
    bool short_circuit;
    uint8_t link_quality_percent;
    int last_service_result;
    uint8_t last_event_count;
    uint16_t last_event_code;
    uint8_t last_isdu_error;
} iolink_master_diagnostics_t;

typedef struct
{
    bool cycle_timer_valid;
    bool awaiting_response;
    uint8_t min_cycle_time_100us;
    uint32_t last_cycle_start_100us;
    uint32_t response_deadline_100us;
} iolink_master_timing_t;

typedef struct
{
    bool valid;
    uint8_t min_cycle_time;
    uint16_t min_cycle_time_100us;
    uint8_t mseq_capability;
    bool isdu_supported;
    uint8_t operate_mseq_code;
    uint8_t preoperate_mseq_code;
    uint8_t revision_id;
    uint8_t pd_in_descriptor;
    uint8_t pd_out_descriptor;
    uint8_t pd_in_len;
    uint8_t pd_out_len;
    uint16_t vendor_id;
    uint32_t device_id;
} iolink_master_device_info_t;

/*
 * Opaque storage budgets keep the public ABI caller-owned and heap-free while
 * giving embedded users a fixed RAM ceiling to audit. Port storage carries the
 * protocol buffers and service state; controller storage only tracks a port
 * array reference plus port count.
 */
#define IOLINK_MASTER_PORT_STORAGE_BUDGET_SIZE 1280U
#define IOLINK_MASTER_CONTROLLER_STORAGE_BUDGET_SIZE 32U
#define IOLINK_MASTER_PORT_STORAGE_SIZE 1280U
#define IOLINK_MASTER_CONTROLLER_STORAGE_SIZE 32U

typedef union
{
    void* align_ptr;
    uint32_t align_u32;
    uint8_t storage[IOLINK_MASTER_PORT_STORAGE_SIZE];
} iolink_master_port_t;

typedef union
{
    void* align_ptr;
    uint32_t align_u32;
    uint8_t storage[IOLINK_MASTER_CONTROLLER_STORAGE_SIZE];
} iolink_master_controller_t;

/*
 * Initializes a port for communication.
 *
 * Lifetime contract: the config is copied into the port, but the PHY is
 * retained BY POINTER (the port stores `phy`, not a copy). The
 * `iolink_phy_api_t` must therefore outlive the port — pass a pointer to
 * storage with at least the port's lifetime, never an automatic/stack
 * temporary. (Passing a stack-local PHY compiles fine but dangles on the next
 * tick.)
 *
 * Returns OK, INVALID_ARG, or a nonzero PHY init error forwarded from phy->init.
 */
int iolink_master_init(iolink_master_port_t* port,
                       const iolink_phy_api_t* phy,
                       const iolink_master_config_t* config);
/* Returns OK when the PHY/config pair has all operations needed for real hardware use. */
int iolink_master_validate_phy_contract(const iolink_phy_api_t* phy,
                                        const iolink_master_config_t* config);
/* Returns OK or INVALID_ARG. */
int iolink_master_restart(iolink_master_port_t* port);
/* Sends one pending startup, preoperate, or operate action. Invalid arguments are ignored. */
void iolink_master_process(iolink_master_port_t* port);
/* Returns decoded frame count, OK when no byte is available, or INVALID_ARG/FRAME/CHECKSUM. */
int iolink_master_poll_rx(iolink_master_port_t* port);
/* Returns OK, PENDING while retrying, INVALID_ARG, or RETRY_LIMIT. */
int iolink_master_on_timeout(iolink_master_port_t* port);
/* Returns poll/process status; response_timeout maps to RESPONSE_TIMEOUT tick event. */
int iolink_master_tick(iolink_master_port_t* port, bool response_timeout);
/* Returns poll/process status for the explicit event, or INVALID_ARG. */
int iolink_master_tick_event(iolink_master_port_t* port, iolink_master_tick_event_t event);
/* Returns poll/process status while applying monotonic 100us cycle pacing. */
int iolink_master_tick_at(iolink_master_port_t* port,
                          iolink_master_tick_event_t event,
                          uint32_t now_100us);
/* Returns OK, INVALID_ARG, FRAME, or CHECKSUM. */
int iolink_master_on_rx(iolink_master_port_t* port, const uint8_t* data, uint8_t len);
/* Returns ERROR for a NULL port, otherwise the current state. */
iolink_master_state_t iolink_master_get_state(const iolink_master_port_t* port);
/* Returns OK with copied data, PENDING when data is not valid yet, or INVALID_ARG/BUFFER_TOO_SMALL. */
int iolink_master_get_pd_in(const iolink_master_port_t* port,
                            uint8_t* buffer,
                            uint8_t buffer_len,
                            uint8_t* out_len);
/* Returns OK or INVALID_ARG. */
int iolink_master_get_od_status(const iolink_master_port_t* port, uint8_t* status);
/* Returns FAILURE for a NULL port, otherwise the current device-status bits. */
uint8_t iolink_master_get_device_status(const iolink_master_port_t* port);
/* Returns OK or INVALID_ARG. */
int iolink_master_get_diagnostics(const iolink_master_port_t* port,
                                  iolink_master_diagnostics_t* diagnostics);
/* Returns OK or INVALID_ARG. Snapshot is read-only scheduler-visible timing state. */
int iolink_master_get_timing(const iolink_master_port_t* port,
                             iolink_master_timing_t* timing);
/* Returns OK, INVALID_ARG, SIO_WRONG_MODE, or SIO_UNSUPPORTED_PHY. */
int iolink_master_set_dq(iolink_master_port_t* port, bool level);
/* Returns OK, INVALID_ARG, SIO_WRONG_MODE, or SIO_UNSUPPORTED_PHY. */
int iolink_master_get_di(const iolink_master_port_t* port, bool* level);
/* Returns OK or INVALID_ARG; switching to IO-Link restarts startup on the port. */
int iolink_master_set_port_mode(iolink_master_port_t* port, iolink_master_port_mode_t mode);
/*
 * Decodes a MinCycleTime/MasterCycleTime octet (Direct Parameter Page 1 byte
 * 0x02) into 100us units per the IO-Link time-base encoding: bits 7-6 select the
 * time base (00 = 0.1ms, 01 = 0.4ms, 10 = 1.6ms) and bits 5-0 the multiplier.
 * Base 00 maps octet value directly to 100us units; base 01 = 6.4ms + n*0.4ms;
 * base 10 = 32.0ms + n*1.6ms. The reserved base 11 falls back to the raw octet.
 */
uint16_t iolink_master_decode_min_cycle_time_100us(uint8_t octet);
/* Composes a Master Command octet from R/W direction, communication channel, and 5-bit address. */
uint8_t iolink_master_encode_master_command(bool read,
                                            iolink_master_mc_channel_t channel,
                                            uint8_t address);
/* Returns true when the Master Command octet is a read (R/W bit set). */
bool iolink_master_mc_is_read(uint8_t mc);
/* Returns the communication channel encoded in a Master Command octet. */
iolink_master_mc_channel_t iolink_master_mc_channel(uint8_t mc);
/* Returns the 5-bit address encoded in a Master Command octet. */
uint8_t iolink_master_mc_address(uint8_t mc);
/* Returns OK, INVALID_ARG, or PARAM_TOO_SHORT. */
int iolink_master_parse_direct_parameter_page1(const uint8_t* page,
                                               uint8_t len,
                                               iolink_master_device_info_t* info);
/* Returns OK, INVALID_ARG, or PARAM_TOO_SHORT. */
int iolink_master_apply_direct_parameter_page1(iolink_master_port_t* port,
                                               const uint8_t* page,
                                               uint8_t len);
/* Returns OK, PENDING when no valid page is stored, or INVALID_ARG. */
int iolink_master_get_device_info(const iolink_master_port_t* port,
                                  iolink_master_device_info_t* info);
/* Returns OK, PENDING, INVALID_ARG, or a PARAM validation error. */
int iolink_master_validate_device_info(const iolink_master_port_t* port);
/* Returns OK, PENDING, INVALID_ARG, or PARAM_M_SEQUENCE for unsupported capabilities. */
int iolink_master_select_config_from_device_info(const iolink_master_device_info_t* info,
                                                 iolink_master_config_t* config);
/* Returns OK, PENDING, INVALID_ARG, or a PARAM validation error for a requested config. */
int iolink_master_validate_config_against_device_info(const iolink_master_device_info_t* info,
                                                      const iolink_master_config_t* config);
/* Returns OK, INVALID_ARG, or BUFFER_TOO_SMALL when len does not match configured PD out. */
int iolink_master_set_pd_out(iolink_master_port_t* port, const uint8_t* data, uint8_t len);
/* Returns OK when complete, PENDING while active, INVALID_ARG, or an ISDU error. */
int iolink_master_read_isdu(iolink_master_port_t* port,
                            uint16_t index,
                            uint8_t subindex,
                            uint8_t* data,
                            uint8_t* len);
/* Returns OK when complete, PENDING while active, INVALID_ARG, or ISDU/PARAM validation errors. */
int iolink_master_read_device_info(iolink_master_port_t* port);
/* Returns OK when complete, PENDING while active, INVALID_ARG, or an ISDU error. */
int iolink_master_write_isdu(iolink_master_port_t* port,
                             uint16_t index,
                             uint8_t subindex,
                             const uint8_t* data,
                             uint8_t len);
/* Returns OK when complete, PENDING while active, INVALID_ARG, or an ISDU error. */
int iolink_master_read_data_storage(iolink_master_port_t* port,
                                    uint8_t* data,
                                    uint8_t* len);
/* Returns OK when complete, PENDING while active, INVALID_ARG, or an ISDU error. */
int iolink_master_write_data_storage(iolink_master_port_t* port,
                                     const uint8_t* data,
                                     uint8_t len);
/* Returns OK after Data Storage download, write, end, and readback verification complete. */
int iolink_master_restore_data_storage(iolink_master_port_t* port,
                                       const uint8_t* data,
                                       uint8_t len);
/* Returns OK when readback matches, PENDING while active, INVALID_ARG, VERIFY_FAILED, or an ISDU error. */
int iolink_master_verify_isdu(iolink_master_port_t* port,
                              uint16_t index,
                              uint8_t subindex,
                              const uint8_t* expected,
                              uint8_t len);
/* Returns OK when Data Storage readback matches, PENDING while active, INVALID_ARG, VERIFY_FAILED, or an ISDU error. */
int iolink_master_verify_data_storage(iolink_master_port_t* port,
                                      const uint8_t* expected,
                                      uint8_t len);
/* Returns OK when complete, PENDING while active, INVALID_ARG, or an ISDU error. */
int iolink_master_read_detailed_device_status(iolink_master_port_t* port,
                                              uint8_t* data,
                                              uint8_t* len);
/* Returns OK when complete, PENDING while active, INVALID_ARG, or an ISDU error. */
int iolink_master_read_event_code(iolink_master_port_t* port, uint16_t* event_code);
/* Returns OK when the device's event-code read completes; this read is the explicit ack policy. */
int iolink_master_ack_event(iolink_master_port_t* port, uint16_t* event_code);
/* Returns OK when complete, PENDING while active, INVALID_ARG, BUFFER_TOO_SMALL, or an ISDU error. */
int iolink_master_read_event_details(iolink_master_port_t* port,
                                     iolink_master_event_t* events,
                                     uint8_t max_events,
                                     uint8_t* out_count);
/* Returns OK when complete, PENDING while active, INVALID_ARG, or an ISDU error. */
int iolink_master_begin_parameter_download(iolink_master_port_t* port);
/* Returns OK when complete, PENDING while active, INVALID_ARG, or an ISDU error. */
int iolink_master_end_parameter_download(iolink_master_port_t* port);
/* Returns OK when complete, PENDING while active, INVALID_ARG, or an ISDU error. */
int iolink_master_begin_parameter_upload(iolink_master_port_t* port);
/* Returns OK when complete, PENDING while active, INVALID_ARG, or an ISDU error. */
int iolink_master_end_parameter_upload(iolink_master_port_t* port);
/* Returns OK when complete, PENDING while active, INVALID_ARG, or an ISDU error. */
int iolink_master_store_parameter_download(iolink_master_port_t* port);
/* Returns OK after download start, write, download end, and readback verification complete. */
int iolink_master_write_parameter_block(iolink_master_port_t* port,
                                        uint16_t index,
                                        uint8_t subindex,
                                        const uint8_t* data,
                                        uint8_t len);
/*
 * Initializes a multi-port controller by init-ing each port from the matching
 * `phys[i]`/`configs[i]`. Same lifetime contract as iolink_master_init: every
 * PHY is retained by pointer, so the `phys` array (and the PHYs it points to)
 * must outlive the controller. The `ports` array must too.
 *
 * Returns OK, INVALID_ARG, or the first per-port init error.
 */
int iolink_master_controller_init(iolink_master_controller_t* controller,
                                  iolink_master_port_t* ports,
                                  uint8_t port_count,
                                  const iolink_phy_api_t* phys,
                                  const iolink_master_config_t* configs);
/* Returns OK or INVALID_ARG. */
int iolink_master_controller_get_port_count(const iolink_master_controller_t* controller,
                                            uint8_t* out_count);
/* Returns OK or INVALID_ARG for NULL/out-of-range access. */
int iolink_master_controller_get_port(iolink_master_controller_t* controller,
                                      uint8_t index,
                                      iolink_master_port_t** out_port);
/* Returns OK, INVALID_ARG, or the first negative per-port tick result. */
int iolink_master_controller_tick(iolink_master_controller_t* controller,
                                  const bool* response_timeouts);
/* Returns OK, INVALID_ARG, or the first negative per-port tick-event result. */
int iolink_master_controller_tick_events(iolink_master_controller_t* controller,
                                         const iolink_master_tick_event_t* events);
/* Returns OK, INVALID_ARG, or the first negative per-port time-aware tick result. */
int iolink_master_controller_tick_at(iolink_master_controller_t* controller,
                                     uint32_t now_100us);
/* Returns OK or INVALID_ARG. Caller owns the hardware timer; output is the next due 100us tick. */
int iolink_master_get_next_tick_time(const iolink_master_port_t* port,
                                     uint32_t now_100us,
                                     uint32_t* out_next_100us);
/* Returns OK or INVALID_ARG. Output is the earliest next due 100us tick across all ports. */
int iolink_master_controller_get_next_tick_time(const iolink_master_controller_t* controller,
                                                uint32_t now_100us,
                                                uint32_t* out_next_100us);

#endif /* IOLINKI_MASTER_MASTER_H */
