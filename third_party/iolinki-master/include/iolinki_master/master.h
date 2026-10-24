/**
 * @file master.h
 * @brief IO-Link Master stack public API.
 *
 * This header is the centerpiece public interface for the IO-Link master
 * stack. It declares the result/enumeration codes, the caller-owned opaque
 * port and controller storage, the configuration and diagnostics structures,
 * and the full set of port-lifecycle, cyclic-scheduling, process-data, ISDU,
 * Data Storage, event, parameter-server and multi-port controller functions.
 * Storage is caller-owned and heap-free so the stack is suitable for
 * bare-metal and safety-critical embedded targets.
 */
#ifndef IOLINKI_MASTER_MASTER_H
#define IOLINKI_MASTER_MASTER_H

#include <stdbool.h>
#include <stdint.h>
#include <stddef.h>

#include "iolinki/config.h"
#include "iolinki/phy.h"

/**
 * @defgroup iolinki_master IO-Link Master Stack (public API)
 * @brief Public API for driving one or more IO-Link master ports.
 * @{
 */

/** @name Enumerations & result codes
 *  @{
 */

/** @brief M-sequence type used on the wire for a port. */
typedef enum
{
    IOLINK_MASTER_M_SEQ_TYPE_0 = 0U,   /**< M-sequence TYPE_0. */
    IOLINK_MASTER_M_SEQ_TYPE_1_1 = 1U, /**< M-sequence TYPE_1_1. */
    IOLINK_MASTER_M_SEQ_TYPE_1_2 = 2U, /**< M-sequence TYPE_1_2. */
    IOLINK_MASTER_M_SEQ_TYPE_1_V = 3U, /**< M-sequence TYPE_1_V (variable). */
    IOLINK_MASTER_M_SEQ_TYPE_2_1 = 4U, /**< M-sequence TYPE_2_1. */
    IOLINK_MASTER_M_SEQ_TYPE_2_2 = 5U, /**< M-sequence TYPE_2_2. */
    IOLINK_MASTER_M_SEQ_TYPE_2_V = 6U, /**< M-sequence TYPE_2_V (variable). */
} iolink_master_m_seq_type_t;

/** @brief Communication state of a master port. */
typedef enum
{
    IOLINK_MASTER_STATE_INACTIVE = 0,   /**< Port not communicating. */
    IOLINK_MASTER_STATE_STARTUP = 1,    /**< Wake-up / establishing communication. */
    IOLINK_MASTER_STATE_PREOPERATE = 2, /**< PREOPERATE: parameterization phase. */
    IOLINK_MASTER_STATE_OPERATE = 3,    /**< OPERATE: cyclic process-data exchange. */
    IOLINK_MASTER_STATE_ERROR = 4       /**< Error state (retry budget exhausted). */
} iolink_master_state_t;

/** @brief Operating mode selected for a master port pin. */
typedef enum
{
    IOLINK_MASTER_PORT_MODE_IOLINK = 0,     /**< IO-Link communication mode. */
    IOLINK_MASTER_PORT_MODE_DI = 1,         /**< Digital input (SIO). */
    IOLINK_MASTER_PORT_MODE_DQ = 2,         /**< Digital output (SIO). */
    IOLINK_MASTER_PORT_MODE_DEACTIVATED = 3 /**< Port deactivated. */
} iolink_master_port_mode_t;

/** @brief Kind of ISDU operation currently in flight. */
typedef enum
{
    IOLINK_MASTER_ISDU_OP_NONE = 0, /**< No ISDU operation active. */
    IOLINK_MASTER_ISDU_OP_READ = 1, /**< ISDU read in progress. */
    IOLINK_MASTER_ISDU_OP_WRITE = 2 /**< ISDU write in progress. */
} iolink_master_isdu_op_t;

/** @brief Severity/type of a decoded device event. */
typedef enum
{
    IOLINK_MASTER_EVENT_TYPE_UNKNOWN = 0U,      /**< Unknown / unclassified event. */
    IOLINK_MASTER_EVENT_TYPE_NOTIFICATION = 1U, /**< Notification-level event. */
    IOLINK_MASTER_EVENT_TYPE_WARNING = 2U,      /**< Warning-level event. */
    IOLINK_MASTER_EVENT_TYPE_ERROR = 3U         /**< Error-level event. */
} iolink_master_event_type_t;

/** @brief Scheduler tick stimulus passed to the cyclic driver. */
typedef enum
{
    IOLINK_MASTER_TICK_NONE = 0,            /**< No timing event this tick. */
    IOLINK_MASTER_TICK_CYCLE_DUE = 1,       /**< A new cycle is due to start. */
    IOLINK_MASTER_TICK_RESPONSE_TIMEOUT = 2 /**< The device response deadline elapsed. */
} iolink_master_tick_event_t;

/**
 * @brief General master result/status codes.
 *
 * @note Several negative codes deliberately share the numeric value -2
 *       (::IOLINK_MASTER_ERR_RETRY_LIMIT, ::IOLINK_MASTER_ERR_FRAME and
 *       ::IOLINK_MASTER_ERR_BUFFER_TOO_SMALL). They are distinct names for the
 *       same code and are documented faithfully; do not "fix" the values.
 */
typedef enum
{
    IOLINK_MASTER_STATUS_OK = 0,             /**< Success. */
    IOLINK_MASTER_STATUS_PENDING = 1,        /**< Operation still in progress. */
    IOLINK_MASTER_ERR_INVALID_ARG = -1,      /**< NULL/invalid argument. */
    IOLINK_MASTER_ERR_RETRY_LIMIT = -2,      /**< Retry budget exhausted (== -2). */
    IOLINK_MASTER_ERR_FRAME = -2,            /**< Malformed frame (shares -2). */
    IOLINK_MASTER_ERR_BUFFER_TOO_SMALL = -2, /**< Caller buffer too small (shares -2). */
    IOLINK_MASTER_ERR_CHECKSUM = -3,         /**< Checksum mismatch. */
    IOLINK_MASTER_ERR_SERVICE = -4,          /**< Service-layer failure. */
    IOLINK_MASTER_ERR_INVALID_STATE = -5,    /**< Call not valid in the current state. */
    IOLINK_MASTER_ERR_UNSUPPORTED_PHY = -6   /**< PHY lacks a required operation. */
} iolink_master_result_t;

/**
 * @brief Result codes for Direct Parameter Page / device-info validation.
 *
 * @note ::IOLINK_MASTER_PARAM_ERR_TOO_SHORT and
 *       ::IOLINK_MASTER_PARAM_ERR_REVISION deliberately share the value -2.
 */
typedef enum
{
    IOLINK_MASTER_PARAM_ERR_TOO_SHORT = -2,  /**< Page/buffer too short (== -2). */
    IOLINK_MASTER_PARAM_ERR_REVISION = -2,   /**< Unsupported revision (shares -2). */
    IOLINK_MASTER_PARAM_ERR_CYCLE_TIME = -3, /**< Cycle-time out of range. */
    IOLINK_MASTER_PARAM_ERR_PD_SIZE = -4,    /**< Process-data size mismatch. */
    IOLINK_MASTER_PARAM_ERR_M_SEQUENCE = -5, /**< Unsupported M-sequence capability. */
    IOLINK_MASTER_PARAM_ERR_VENDOR_ID = -6,  /**< VendorID mismatch. */
    IOLINK_MASTER_PARAM_ERR_DEVICE_ID = -7,  /**< DeviceID mismatch. */
    /**
     * Reserved Direct Parameter Page 1 ProcessData descriptor (Table B.6):
     * BYTE = 1 with Length 0/1, or BYTE = 0 with Length 17..31.
     */
    IOLINK_MASTER_PARAM_ERR_PD_DESCRIPTOR = -8
} iolink_master_parameter_result_t;

/**
 * @brief Device-identity inspection level, per the IO-Link port configuration
 *        model.
 *
 * NO_CHECK establishes communication without comparing the device identity.
 * TYPE_COMP requires the connected device's VendorID and DeviceID to match the
 * configured expected values (a type-compatible device). IDENTICAL additionally
 * requires the device SerialNumber to match; the SerialNumber leg is not yet
 * wired here (it lives in ISDU index 0x0015, not Direct Parameter Page 1), so
 * IDENTICAL currently enforces the same VendorID/DeviceID check as TYPE_COMP.
 */
typedef enum
{
    IOLINK_MASTER_INSPECTION_NO_CHECK = 0,  /**< Link up without identity comparison. */
    IOLINK_MASTER_INSPECTION_TYPE_COMP = 1, /**< Require matching VendorID/DeviceID. */
    IOLINK_MASTER_INSPECTION_IDENTICAL = 2  /**< As TYPE_COMP (SerialNumber leg not yet wired). */
} iolink_master_inspection_level_t;

/** @brief Result codes specific to ISDU services. */
typedef enum
{
    IOLINK_MASTER_ISDU_ERR_BUFFER_TOO_SMALL = -2, /**< Caller buffer too small. */
    IOLINK_MASTER_ISDU_ERR_BUSY = -3,             /**< Another ISDU op is active. */
    IOLINK_MASTER_ISDU_ERR_DEVICE = -4,           /**< Device reported an ISDU error. */
    IOLINK_MASTER_ISDU_ERR_INVALID_STATE = -5,    /**< ISDU not valid in current state. */
    IOLINK_MASTER_ISDU_ERR_VERIFY_FAILED = -6     /**< Readback did not match expected. */
} iolink_master_isdu_result_t;

/** @brief Result codes specific to SIO (DI/DQ) operations. */
typedef enum
{
    IOLINK_MASTER_SIO_ERR_WRONG_MODE = -2,     /**< Port not in the required DI/DQ mode. */
    IOLINK_MASTER_SIO_ERR_UNSUPPORTED_PHY = -3 /**< PHY does not support the SIO line op. */
} iolink_master_sio_result_t;

/**
 * @brief Master Command communication channel, per the IO-Link Master Command
 *        octet layout (`IOLINK_MC_COMM_CHANNEL_MASK`).
 *
 * The channel selects which logical page/diagnosis/ISDU register a Master
 * Command addresses.
 */
typedef enum
{
    IOLINK_MASTER_MC_CHANNEL_PROCESS = 0U,   /**< Process channel. */
    IOLINK_MASTER_MC_CHANNEL_PAGE = 1U,      /**< Page channel. */
    IOLINK_MASTER_MC_CHANNEL_DIAGNOSIS = 2U, /**< Diagnosis channel. */
    IOLINK_MASTER_MC_CHANNEL_ISDU = 3U       /**< ISDU channel. */
} iolink_master_mc_channel_t;

/**
 * @brief 5-bit address which, with a process-channel write, encodes the legacy
 *        Type-0 octet 0x0F (IOLINK_MC_TRANSITION_COMMAND).
 *
 * That octet is not the V1.1.5 DeviceOperate transition. DeviceOperate is a
 * page-channel write of IOLINK_CMD_DEVICE_OPERATE (0x99) to Direct Parameter
 * address 0x00.
 */
#define IOLINK_MASTER_MC_TRANSITION_ADDR 0x0FU

/** @} */ /* end of Enumerations & result codes */

/** @name Configuration & callbacks
 *  @{
 */

/** @brief A single decoded device event. */
typedef struct
{
    uint8_t qualifier;               /**< Raw event qualifier octet. */
    iolink_master_event_type_t type; /**< Decoded event severity/type. */
    uint16_t code;                   /**< Event code reported by the device. */
} iolink_master_event_t;

/**
 * @brief Optional event-dispatch callbacks.
 *
 * When set, they turn event handling from a poll-only model (read
 * `diagnostics.event_pending` yourself) into a dispatch:
 * `event_pending_handler` fires on the rising edge of the OD Event flag during a
 * cyclic response, prompting the application to read event details;
 * `event_handler` fires once per decoded event from
 * `iolink_master_read_event_details`. Both may be NULL to keep poll-only
 * behavior. `event_user` is passed through unchanged.
 */
typedef void (*iolink_master_event_pending_cb_t)(void* user);
/** @brief Per-event dispatch callback; see ::iolink_master_event_pending_cb_t. */
typedef void (*iolink_master_event_cb_t)(void* user, const iolink_master_event_t* event);

/** @brief Per-port configuration, copied into the port on init. */
typedef struct
{
    iolink_master_port_mode_t port_mode;   /**< Operating mode (IO-Link/DI/DQ/deactivated). */
    iolink_master_m_seq_type_t m_seq_type; /**< M-sequence type to drive. */
    iolink_baudrate_t baudrate;            /**< COM baudrate (start point if auto). */
    uint8_t min_cycle_time;                /**< Minimum cycle time (raw octet encoding). */
    uint8_t pd_in_len;                     /**< Configured input process-data length, in bytes. */
    uint8_t pd_out_len;                    /**< Configured output process-data length, in bytes. */
    /**
     * If true (spec behaviour, 7.3.2.2 / Figure 36), every wake-up request is
     * followed by one TYPE_0 test message at COM3, then COM2, then COM1 until
     * the device answers. If false, only @c baudrate is tried after each
     * wake-up; use this only for a PHY that cannot run the other rates.
     */
    bool auto_baudrate;
    bool validate_device_info;             /**< If true, validate device identity/config. */
    iolink_master_inspection_level_t inspection_level; /**< Identity inspection level to enforce. */
    uint16_t expected_vendor_id;    /**< Expected VendorID for identity checks. */
    uint32_t expected_device_id;    /**< Expected DeviceID for identity checks. */
    uint8_t response_timeout_100us; /**< Device response deadline, in 100us units. */
    /**
     * ISDU response watchdog (ISDUTime, 7.3.6.3 / Table 102), in 100us units.
     * 0 selects the default of 50000 (5 s). It bounds how long the master keeps
     * polling the ISDU channel while the device answers "busy"/"no service".
     */
    uint32_t isdu_timeout_100us;
    /**
     * Master message delay T_DMT (Table 42), in bit times of the rate of the
     * next message. It is applied after T_REN following a wake-up, and between
     * the test messages of the COM3, COM2, COM1 scan (7.3.2.2). The spec range
     * is 27..37 T_BIT; 0 selects the default of 32.
     */
    uint8_t t_dmt_tbit;
    /**
     * Wake-up retry delay T_DWU (Table 42), in 100us units: the wait after a
     * failed establish attempt (no answer at any tried rate) before the next
     * wake-up request (Figure 32). The spec range is 30..50 ms; 0 selects the
     * default of 400 (40 ms).
     */
    uint32_t t_dwu_100us;
    /**
     * Wake-up retries n_WU (Table 42) in one wake-up retry sequence: the master
     * makes up to n_WU + 1 wake-up requests, each followed by the rate scan,
     * before it reports the sequence as failed (Figure 33, Figure 36). 0
     * selects the spec value 2, which is the only conformant value.
     */
    uint8_t wake_retry_limit;
    /**
     * Device detection time T_SD (Table 42), in 100us units: after a failed
     * wake-up retry sequence the PHY is set inactive and the next sequence
     * starts no earlier than T_SD later (7.3.2.2). The spec range is 0.5..1 s;
     * 0 selects the default of 5000 (500 ms).
     */
    uint32_t t_sd_100us;
    void* event_user; /**< Opaque user pointer passed to event callbacks. */
    iolink_master_event_pending_cb_t
        event_pending_handler;              /**< Event-pending edge callback (may be NULL). */
    iolink_master_event_cb_t event_handler; /**< Per-decoded-event callback (may be NULL). */
    int (*set_mode_checked)(
        iolink_phy_mode_t mode); /**< Checked PHY mode setter (hardware-contract op). */
    int (*set_baudrate_checked)(
        iolink_baudrate_t baudrate);   /**< Checked PHY baudrate setter (hardware-contract op). */
    int (*read_cq_line_checked)(void); /**< Checked C/Q line read (hardware-contract op). */
    int (*flush_rx)(void);             /**< Flush the PHY receive path. */
    int (*prepare_tx)(void);           /**< Prepare the PHY for transmit. */
    int (*prepare_rx)(void);           /**< Prepare the PHY for receive. */
    int (*read_cq_line)(void);         /**< Raw C/Q line read. */
    int (*wake_up)(void);              /**< Emit a wake-up request (NULL uses default pattern). */
} iolink_master_config_t;

/** @} */ /* end of Configuration & callbacks */

/** @name Diagnostics, timing & device info
 *  @{
 */

/** @brief Runtime diagnostics snapshot for a port. */
typedef struct
{
    uint8_t od_status;                /**< Last reply CKS octet (A.1.5). */
    uint8_t device_status;            /**< Last DeviceStatus octet read via ISDU (0x0024). */
    bool event_pending;               /**< True while the reply Event flag (CKS bit 7) is set. */
    uint8_t rx_retry_count;           /**< Current consecutive RX retry count. */
    uint32_t checksum_errors;         /**< Cumulative checksum errors. */
    uint32_t send_errors;             /**< Cumulative transmit errors. */
    uint32_t response_timeouts;       /**< Cumulative response timeouts. */
    uint32_t cycle_slips;             /**< Cumulative cycle slips (missed deadlines). */
    uint32_t last_cycle_jitter_100us; /**< Jitter of the last cycle, in 100us units. */
    uint32_t max_cycle_jitter_100us;  /**< Peak observed cycle jitter, in 100us units. */
    int supply_voltage_mv;            /**< Measured supply voltage, in millivolts. */
    bool short_circuit;               /**< True if a short-circuit condition is detected. */
    uint8_t link_quality_percent;     /**< Estimated link quality, 0-100 percent. */
    int last_service_result;          /**< Result of the last acyclic service. */
    uint8_t last_event_count;         /**< Number of events in the last event read. */
    uint16_t last_event_code;         /**< Most recent decoded event code. */
    uint16_t last_isdu_error;         /**< Most recent ISDU ErrorType (ErrorCode<<8|AdditionalCode). */
    uint32_t establish_failures; /**< Failed wake-up retry sequences (DL_Mode INACTIVE, 7.3.2.2). */
} iolink_master_diagnostics_t;

/** @brief Read-only scheduler-visible timing snapshot for a port. */
typedef struct
{
    bool cycle_timer_valid;           /**< True once the cycle timer has a valid start. */
    bool awaiting_response;           /**< True while waiting on a device response. */
    uint8_t min_cycle_time_100us;     /**< Minimum cycle time, in 100us units. */
    uint32_t last_cycle_start_100us;  /**< Timestamp of the last cycle start, in 100us units. */
    uint32_t response_deadline_100us; /**< Response deadline timestamp, in 100us units. */
} iolink_master_timing_t;

/** @brief Decoded device identification and capabilities. */
typedef struct
{
    bool valid;                    /**< True when the fields hold a decoded page. */
    uint8_t min_cycle_time;        /**< MinCycleTime raw octet. */
    uint16_t min_cycle_time_100us; /**< Decoded MinCycleTime, in 100us units. */
    uint8_t mseq_capability;       /**< M-sequenceCapability octet. */
    bool isdu_supported;           /**< True if the device supports ISDU. */
    uint8_t operate_mseq_code;     /**< M-sequence code used in OPERATE. */
    uint8_t preoperate_mseq_code;  /**< M-sequence code used in PREOPERATE. */
    uint8_t revision_id;           /**< RevisionID octet. */
    uint8_t pd_in_descriptor;      /**< Input process-data descriptor octet. */
    uint8_t pd_out_descriptor;     /**< Output process-data descriptor octet. */
    uint8_t pd_in_len;             /**< Decoded input process-data length, in bytes. */
    uint8_t pd_out_len;            /**< Decoded output process-data length, in bytes. */
    uint16_t vendor_id;            /**< Device VendorID. */
    uint32_t device_id;            /**< Device DeviceID. */
} iolink_master_device_info_t;

/** @} */ /* end of Diagnostics, timing & device info */

/** @name Opaque storage
 *  @{
 */

/**
 * @brief Opaque storage budgets.
 *
 * These budgets keep the public ABI caller-owned and heap-free while giving
 * embedded users a fixed RAM ceiling to audit. Port storage carries the
 * protocol buffers and service state; controller storage only tracks a port
 * array reference plus port count.
 */
#define IOLINK_MASTER_PORT_STORAGE_BUDGET_SIZE \
    1312U /**< Auditing budget for port storage, in bytes. */
#define IOLINK_MASTER_CONTROLLER_STORAGE_BUDGET_SIZE \
    32U /**< Auditing budget for controller storage, in bytes. */
#define IOLINK_MASTER_PORT_STORAGE_SIZE 1312U /**< Actual port opaque storage size, in bytes. */
#define IOLINK_MASTER_CONTROLLER_STORAGE_SIZE \
    32U /**< Actual controller opaque storage size, in bytes. */

/** @brief Caller-owned opaque storage for one master port (alignment-safe union). */
typedef union
{
    void* align_ptr;    /**< Pointer member forcing pointer alignment. */
    uint32_t align_u32; /**< 32-bit member forcing word alignment. */
    uint8_t storage[IOLINK_MASTER_PORT_STORAGE_SIZE]; /**< Raw backing bytes for the private port
                                                         state. */
} iolink_master_port_t;

/** @brief Caller-owned opaque storage for a multi-port controller (alignment-safe union). */
typedef union
{
    void* align_ptr;    /**< Pointer member forcing pointer alignment. */
    uint32_t align_u32; /**< 32-bit member forcing word alignment. */
    uint8_t storage[IOLINK_MASTER_CONTROLLER_STORAGE_SIZE]; /**< Raw backing bytes for the private
                                                               controller state. */
} iolink_master_controller_t;

/** @} */ /* end of Opaque storage */

/** @name Port lifecycle
 *  @{
 */

/**
 * @brief Initialize a port for communication.
 *
 * Lifetime contract: the config is copied into the port, but the PHY is
 * retained BY POINTER (the port stores `phy`, not a copy). The
 * `iolink_phy_api_t` must therefore outlive the port — pass a pointer to
 * storage with at least the port's lifetime, never an automatic/stack
 * temporary. (Passing a stack-local PHY compiles fine but dangles on the next
 * tick.)
 *
 * @param port    Caller-owned opaque port storage to initialize.
 * @param phy     PHY API, retained by pointer; must outlive the port.
 * @param config  Configuration to copy into the port.
 * @return ::IOLINK_MASTER_STATUS_OK, ::IOLINK_MASTER_ERR_INVALID_ARG, or a
 *         nonzero PHY init error forwarded from phy->init.
 */
int iolink_master_init(iolink_master_port_t* port, const iolink_phy_api_t* phy,
                       const iolink_master_config_t* config);
/**
 * @brief Validate that a PHY/config pair is complete enough for real hardware.
 *
 * @param phy     PHY API to check.
 * @param config  Configuration to check against.
 * @return ::IOLINK_MASTER_STATUS_OK when the PHY/config pair has all operations
 *         needed for real hardware use, otherwise an error code.
 */
int iolink_master_validate_phy_contract(const iolink_phy_api_t* phy,
                                        const iolink_master_config_t* config);
/**
 * @brief Restart a port back into startup.
 *
 * @param port  Port to restart.
 * @return ::IOLINK_MASTER_STATUS_OK or ::IOLINK_MASTER_ERR_INVALID_ARG.
 */
int iolink_master_restart(iolink_master_port_t* port);
/**
 * @brief Send one pending startup, preoperate, or operate action.
 *
 * Invalid arguments are ignored.
 *
 * @param port  Port to advance.
 */
void iolink_master_process(iolink_master_port_t* port);
/**
 * @brief Poll the PHY receive path and decode any available frame.
 *
 * @param port  Port to poll.
 * @return Decoded frame count, ::IOLINK_MASTER_STATUS_OK when no byte is
 *         available, or ::IOLINK_MASTER_ERR_INVALID_ARG /
 *         ::IOLINK_MASTER_ERR_FRAME / ::IOLINK_MASTER_ERR_CHECKSUM.
 */
int iolink_master_poll_rx(iolink_master_port_t* port);
/**
 * @brief Handle a response-timeout event for a port.
 *
 * During STARTUP a timeout ends the current test message without an answer
 * (Table 46, AwaitReply_1). The port moves on to the next rate of the
 * COM3, COM2, COM1 scan, or issues the next wake-up request, or, after
 * n_WU + 1 failed wake-up requests, reports the failed sequence with
 * ::IOLINK_MASTER_ERR_RETRY_LIMIT, sets the PHY inactive and starts a new
 * sequence on a later tick (7.3.2.2, Figure 33, Figure 36). The port stays in
 * STARTUP; it does not latch ERROR.
 *
 * This entry point has no time base, so the T_DMT, T_DWU and T_SD waits are
 * skipped. Use ::iolink_master_tick_at for spec timing.
 *
 * @param port  Port whose response deadline elapsed.
 * @return ::IOLINK_MASTER_STATUS_OK, ::IOLINK_MASTER_STATUS_PENDING while
 *         retrying, ::IOLINK_MASTER_ERR_INVALID_ARG, or
 *         ::IOLINK_MASTER_ERR_RETRY_LIMIT when a wake-up retry sequence failed.
 */
int iolink_master_on_timeout(iolink_master_port_t* port);

/** @} */ /* end of Port lifecycle */

/** @name Cyclic scheduling & I/O
 *  @{
 */

/**
 * @brief Drive one scheduler tick, optionally signalling a response timeout.
 *
 * @param port              Port to tick.
 * @param response_timeout  True to signal a response-timeout tick event.
 * @return Poll/process status; @p response_timeout maps to the
 *         ::IOLINK_MASTER_TICK_RESPONSE_TIMEOUT tick event.
 */
int iolink_master_tick(iolink_master_port_t* port, bool response_timeout);
/**
 * @brief Drive one scheduler tick for an explicit tick event.
 *
 * @param port   Port to tick.
 * @param event  Explicit tick stimulus.
 * @return Poll/process status for the explicit event, or
 *         ::IOLINK_MASTER_ERR_INVALID_ARG.
 */
int iolink_master_tick_event(iolink_master_port_t* port, iolink_master_tick_event_t event);
/**
 * @brief Drive one scheduler tick with a monotonic timestamp for pacing.
 *
 * @param port      Port to tick.
 * @param event     Tick stimulus.
 * @param now_100us Current monotonic time, in 100us units.
 * @return Poll/process status while applying monotonic 100us cycle pacing.
 */
int iolink_master_tick_at(iolink_master_port_t* port, iolink_master_tick_event_t event,
                          uint32_t now_100us);
/**
 * @brief Feed a received frame directly into a port.
 *
 * @param port  Port to receive on.
 * @param data  Received frame bytes.
 * @param len   Number of bytes in @p data.
 * @return ::IOLINK_MASTER_STATUS_OK, ::IOLINK_MASTER_ERR_INVALID_ARG,
 *         ::IOLINK_MASTER_ERR_FRAME, or ::IOLINK_MASTER_ERR_CHECKSUM.
 */
int iolink_master_on_rx(iolink_master_port_t* port, const uint8_t* data, uint8_t len);
/**
 * @brief Get the current communication state of a port.
 *
 * @param port  Port to query.
 * @return ::IOLINK_MASTER_STATE_ERROR for a NULL port, otherwise the current
 *         state.
 */
iolink_master_state_t iolink_master_get_state(const iolink_master_port_t* port);
/**
 * @brief Copy the latest input process data out of a port.
 *
 * @param port        Port to read from.
 * @param[out] buffer Destination for the input process data.
 * @param buffer_len  Capacity of @p buffer, in bytes.
 * @param[out] out_len Filled with the number of bytes copied.
 * @return ::IOLINK_MASTER_STATUS_OK with copied data, ::IOLINK_MASTER_STATUS_PENDING
 *         when data is not valid yet, or
 *         ::IOLINK_MASTER_ERR_INVALID_ARG / ::IOLINK_MASTER_ERR_BUFFER_TOO_SMALL.
 */
int iolink_master_get_pd_in(const iolink_master_port_t* port, uint8_t* buffer, uint8_t buffer_len,
                            uint8_t* out_len);
/**
 * @brief Read the last on-request-data status octet.
 *
 * @param port         Port to query.
 * @param[out] status  Filled with the OD status octet.
 * @return ::IOLINK_MASTER_STATUS_OK or ::IOLINK_MASTER_ERR_INVALID_ARG.
 */
int iolink_master_get_od_status(const iolink_master_port_t* port, uint8_t* status);
/**
 * @brief Get the current device-status bits.
 *
 * @param port  Port to query.
 * @return 0xFF (failure sentinel) for a NULL port, otherwise the current
 *         device-status bits.
 */
uint8_t iolink_master_get_device_status(const iolink_master_port_t* port);
/**
 * @brief Copy the diagnostics snapshot out of a port.
 *
 * @param port              Port to query.
 * @param[out] diagnostics  Destination diagnostics structure.
 * @return ::IOLINK_MASTER_STATUS_OK or ::IOLINK_MASTER_ERR_INVALID_ARG.
 */
int iolink_master_get_diagnostics(const iolink_master_port_t* port,
                                  iolink_master_diagnostics_t* diagnostics);
/**
 * @brief Copy the read-only scheduler-visible timing snapshot out of a port.
 *
 * @param port         Port to query.
 * @param[out] timing  Destination timing structure.
 * @return ::IOLINK_MASTER_STATUS_OK or ::IOLINK_MASTER_ERR_INVALID_ARG.
 */
int iolink_master_get_timing(const iolink_master_port_t* port, iolink_master_timing_t* timing);

/** @} */ /* end of Cyclic scheduling & I/O */

/** @name Process data & SIO
 *  @{
 */

/**
 * @brief Drive the DQ output line when the port is in DQ mode.
 *
 * @param port   Port in DQ mode.
 * @param level  Output level to drive.
 * @return ::IOLINK_MASTER_STATUS_OK, ::IOLINK_MASTER_ERR_INVALID_ARG,
 *         ::IOLINK_MASTER_SIO_ERR_WRONG_MODE, or
 *         ::IOLINK_MASTER_SIO_ERR_UNSUPPORTED_PHY.
 */
int iolink_master_set_dq(iolink_master_port_t* port, bool level);
/**
 * @brief Read the DI input line when the port is in DI mode.
 *
 * @param port         Port in DI mode.
 * @param[out] level   Filled with the sampled input level.
 * @return ::IOLINK_MASTER_STATUS_OK, ::IOLINK_MASTER_ERR_INVALID_ARG,
 *         ::IOLINK_MASTER_SIO_ERR_WRONG_MODE, or
 *         ::IOLINK_MASTER_SIO_ERR_UNSUPPORTED_PHY.
 */
int iolink_master_get_di(const iolink_master_port_t* port, bool* level);
/**
 * @brief Change a port's operating mode.
 *
 * @param port  Port to reconfigure.
 * @param mode  New operating mode.
 * @return ::IOLINK_MASTER_STATUS_OK or ::IOLINK_MASTER_ERR_INVALID_ARG;
 *         switching to IO-Link restarts startup on the port.
 */
int iolink_master_set_port_mode(iolink_master_port_t* port, iolink_master_port_mode_t mode);

/** @} */ /* end of Process data & SIO */

/** @name Direct Parameter Page / device info decoding
 *  @{
 */

/**
 * @brief Decode a MinCycleTime/MasterCycleTime octet into 100us units.
 *
 * Decodes Direct Parameter Page 1 byte 0x02 per the IO-Link time-base
 * encoding: bits 7-6 select the time base (00 = 0.1ms, 01 = 0.4ms,
 * 10 = 1.6ms) and bits 5-0 the multiplier. Base 00 maps the octet value
 * directly to 100us units; base 01 = 6.4ms + n*0.4ms; base 10 = 32.0ms +
 * n*1.6ms. The reserved base 11 falls back to the raw octet.
 *
 * @param octet  The MinCycleTime octet to decode.
 * @return The decoded cycle time, in 100us units.
 */
uint16_t iolink_master_decode_min_cycle_time_100us(uint8_t octet);
/**
 * @brief Compose a Master Command octet.
 *
 * @param read     True for a read command (sets the R/W bit).
 * @param channel  Communication channel to address.
 * @param address  5-bit register address.
 * @return The composed Master Command octet from the R/W direction,
 *         communication channel, and 5-bit address.
 */
uint8_t iolink_master_encode_master_command(bool read, iolink_master_mc_channel_t channel,
                                            uint8_t address);
/**
 * @brief Test whether a Master Command octet is a read.
 *
 * @param mc  Master Command octet.
 * @return true when the octet is a read (R/W bit set).
 */
bool iolink_master_mc_is_read(uint8_t mc);
/**
 * @brief Extract the communication channel from a Master Command octet.
 *
 * @param mc  Master Command octet.
 * @return The communication channel encoded in @p mc.
 */
iolink_master_mc_channel_t iolink_master_mc_channel(uint8_t mc);
/**
 * @brief Extract the 5-bit address from a Master Command octet.
 *
 * @param mc  Master Command octet.
 * @return The 5-bit address encoded in @p mc.
 */
uint8_t iolink_master_mc_address(uint8_t mc);
/**
 * @brief Parse a Direct Parameter Page 1 buffer into device info.
 *
 * @param page       Raw Direct Parameter Page 1 bytes.
 * @param len        Number of bytes in @p page.
 * @param[out] info  Destination device-info structure.
 * @return ::IOLINK_MASTER_STATUS_OK, ::IOLINK_MASTER_ERR_INVALID_ARG, or
 *         ::IOLINK_MASTER_PARAM_ERR_TOO_SHORT.
 */
int iolink_master_parse_direct_parameter_page1(const uint8_t* page, uint8_t len,
                                               iolink_master_device_info_t* info);
/**
 * @brief Parse and apply a Direct Parameter Page 1 buffer to a port.
 *
 * @param port  Port whose device info is updated.
 * @param page  Raw Direct Parameter Page 1 bytes.
 * @param len   Number of bytes in @p page.
 * @return ::IOLINK_MASTER_STATUS_OK, ::IOLINK_MASTER_ERR_INVALID_ARG, or
 *         ::IOLINK_MASTER_PARAM_ERR_TOO_SHORT.
 */
int iolink_master_apply_direct_parameter_page1(iolink_master_port_t* port, const uint8_t* page,
                                               uint8_t len);
/**
 * @brief Copy the stored device info out of a port.
 *
 * @param port       Port to query.
 * @param[out] info  Destination device-info structure.
 * @return ::IOLINK_MASTER_STATUS_OK, ::IOLINK_MASTER_STATUS_PENDING when no
 *         valid page is stored, or ::IOLINK_MASTER_ERR_INVALID_ARG.
 */
int iolink_master_get_device_info(const iolink_master_port_t* port,
                                  iolink_master_device_info_t* info);

#define IOLINK_MASTER_GATEWAY_LINE_MAX 128U

/**
 * @brief Format one NUL-terminated gateway line, including its trailing newline.
 *
 * The line is `iolinki-gw/1 <port> <vendor-4hex> <device-8hex> <pd-hex-or-dash>`.
 * @p info->valid must be true. @p pd_in may be NULL only when @p pd_in_len is 0.
 * @p pd_in_len above IOLINK_PD_IN_MAX_SIZE is invalid.
 *
 * @return ::IOLINK_MASTER_STATUS_OK, ::IOLINK_MASTER_ERR_INVALID_ARG, or
 *         ::IOLINK_MASTER_ERR_BUFFER_TOO_SMALL.
 */
int iolink_master_format_gateway_line(uint8_t port_index,
                                      const iolink_master_device_info_t* info,
                                      const uint8_t* pd_in,
                                      uint8_t pd_in_len,
                                      char* out,
                                      size_t out_len);

/**
 * @brief Collect device info and PD-in from a port and format one gateway line.
 *
 * @return ::IOLINK_MASTER_STATUS_PENDING when identity is not valid yet.
 *         A known device with no valid process data yet is reported with pd "-".
 *         Otherwise returns the ::iolink_master_format_gateway_line result.
 */
int iolink_master_write_gateway_line(const iolink_master_port_t* port,
                                     uint8_t port_index,
                                     char* out,
                                     size_t out_len);
/**
 * @brief Validate a port's stored device info against its configuration.
 *
 * @param port  Port to validate.
 * @return ::IOLINK_MASTER_STATUS_OK, ::IOLINK_MASTER_STATUS_PENDING,
 *         ::IOLINK_MASTER_ERR_INVALID_ARG, or a PARAM validation error.
 */
int iolink_master_validate_device_info(const iolink_master_port_t* port);
/**
 * @brief Derive a workable config from decoded device info.
 *
 * @param info          Decoded device info to select from.
 * @param[out] config   Filled with a compatible configuration.
 * @return ::IOLINK_MASTER_STATUS_OK, ::IOLINK_MASTER_STATUS_PENDING,
 *         ::IOLINK_MASTER_ERR_INVALID_ARG, or ::IOLINK_MASTER_PARAM_ERR_M_SEQUENCE
 *         for unsupported capabilities.
 */
int iolink_master_select_config_from_device_info(const iolink_master_device_info_t* info,
                                                 iolink_master_config_t* config);
/**
 * @brief Validate a requested config against decoded device info.
 *
 * @param info    Decoded device info.
 * @param config  Requested configuration to validate.
 * @return ::IOLINK_MASTER_STATUS_OK, ::IOLINK_MASTER_STATUS_PENDING,
 *         ::IOLINK_MASTER_ERR_INVALID_ARG, or a PARAM validation error for a
 *         requested config.
 */
int iolink_master_validate_config_against_device_info(const iolink_master_device_info_t* info,
                                                      const iolink_master_config_t* config);
/**
 * @brief Stage output process data for the next cycle.
 *
 * @param port  Port to write to.
 * @param data  Output process-data bytes.
 * @param len   Number of bytes in @p data.
 * @return ::IOLINK_MASTER_STATUS_OK, ::IOLINK_MASTER_ERR_INVALID_ARG, or
 *         ::IOLINK_MASTER_ERR_BUFFER_TOO_SMALL when @p len does not match the
 *         configured PD out length.
 */
int iolink_master_set_pd_out(iolink_master_port_t* port, const uint8_t* data, uint8_t len);

/** @} */ /* end of Direct Parameter Page / device info decoding */

/** @name ISDU & Data Storage services
 *  @{
 */

/**
 * @brief Read an ISDU object from the device.
 *
 * @param port        Port to operate on.
 * @param index       ISDU index.
 * @param subindex    ISDU subindex.
 * @param[out] data   Destination for the read payload.
 * @param[in,out] len On input the buffer capacity; on output the bytes read.
 * @return ::IOLINK_MASTER_STATUS_OK when complete, ::IOLINK_MASTER_STATUS_PENDING
 *         while active, ::IOLINK_MASTER_ERR_INVALID_ARG, or an ISDU error.
 */
int iolink_master_read_isdu(iolink_master_port_t* port, uint16_t index, uint8_t subindex,
                            uint8_t* data, uint8_t* len);
/**
 * @brief Read the device info via ISDU services.
 *
 * @param port  Port to operate on.
 * @return ::IOLINK_MASTER_STATUS_OK when complete, ::IOLINK_MASTER_STATUS_PENDING
 *         while active, ::IOLINK_MASTER_ERR_INVALID_ARG, or ISDU/PARAM
 *         validation errors.
 */
int iolink_master_read_device_info(iolink_master_port_t* port);
/**
 * @brief Write an ISDU object to the device.
 *
 * @param port      Port to operate on.
 * @param index     ISDU index.
 * @param subindex  ISDU subindex.
 * @param data      Payload to write.
 * @param len       Number of bytes in @p data.
 * @return ::IOLINK_MASTER_STATUS_OK when complete, ::IOLINK_MASTER_STATUS_PENDING
 *         while active, ::IOLINK_MASTER_ERR_INVALID_ARG, or an ISDU error.
 */
int iolink_master_write_isdu(iolink_master_port_t* port, uint16_t index, uint8_t subindex,
                             const uint8_t* data, uint8_t len);
/**
 * @brief Read the device Data Storage object.
 *
 * @param port        Port to operate on.
 * @param[out] data   Destination for the Data Storage payload.
 * @param[in,out] len On input the buffer capacity; on output the bytes read.
 * @return ::IOLINK_MASTER_STATUS_OK when complete, ::IOLINK_MASTER_STATUS_PENDING
 *         while active, ::IOLINK_MASTER_ERR_INVALID_ARG, or an ISDU error.
 */
int iolink_master_read_data_storage(iolink_master_port_t* port, uint8_t* data, uint8_t* len);
/**
 * @brief Write the device Data Storage object.
 *
 * @param port  Port to operate on.
 * @param data  Data Storage payload to write.
 * @param len   Number of bytes in @p data.
 * @return ::IOLINK_MASTER_STATUS_OK when complete, ::IOLINK_MASTER_STATUS_PENDING
 *         while active, ::IOLINK_MASTER_ERR_INVALID_ARG, or an ISDU error.
 */
int iolink_master_write_data_storage(iolink_master_port_t* port, const uint8_t* data, uint8_t len);
/**
 * @brief Restore a Data Storage image with readback verification.
 *
 * @param port  Port to operate on.
 * @param data  Data Storage image to restore.
 * @param len   Number of bytes in @p data.
 * @return ::IOLINK_MASTER_STATUS_OK after Data Storage download, write, end,
 *         and readback verification complete.
 */
int iolink_master_restore_data_storage(iolink_master_port_t* port, const uint8_t* data,
                                       uint8_t len);
/**
 * @brief Read back an ISDU object and compare it to an expected value.
 *
 * @param port      Port to operate on.
 * @param index     ISDU index.
 * @param subindex  ISDU subindex.
 * @param expected  Expected payload to compare against.
 * @param len       Number of bytes in @p expected.
 * @return ::IOLINK_MASTER_STATUS_OK when readback matches,
 *         ::IOLINK_MASTER_STATUS_PENDING while active,
 *         ::IOLINK_MASTER_ERR_INVALID_ARG, ::IOLINK_MASTER_ISDU_ERR_VERIFY_FAILED,
 *         or an ISDU error.
 */
int iolink_master_verify_isdu(iolink_master_port_t* port, uint16_t index, uint8_t subindex,
                              const uint8_t* expected, uint8_t len);
/**
 * @brief Read back Data Storage and compare it to an expected image.
 *
 * @param port      Port to operate on.
 * @param expected  Expected Data Storage image.
 * @param len       Number of bytes in @p expected.
 * @return ::IOLINK_MASTER_STATUS_OK when Data Storage readback matches,
 *         ::IOLINK_MASTER_STATUS_PENDING while active,
 *         ::IOLINK_MASTER_ERR_INVALID_ARG, ::IOLINK_MASTER_ISDU_ERR_VERIFY_FAILED,
 *         or an ISDU error.
 */
int iolink_master_verify_data_storage(iolink_master_port_t* port, const uint8_t* expected,
                                      uint8_t len);
/**
 * @brief Read the device's detailed status via ISDU.
 *
 * @param port        Port to operate on.
 * @param[out] data   Destination for the detailed status payload.
 * @param[in,out] len On input the buffer capacity; on output the bytes read.
 * @return ::IOLINK_MASTER_STATUS_OK when complete, ::IOLINK_MASTER_STATUS_PENDING
 *         while active, ::IOLINK_MASTER_ERR_INVALID_ARG, or an ISDU error.
 */
int iolink_master_read_detailed_device_status(iolink_master_port_t* port, uint8_t* data,
                                              uint8_t* len);

/** @} */ /* end of ISDU & Data Storage services */

/** @name Event services
 *  @{
 */

/**
 * @brief Read the device's current event code.
 *
 * @param port             Port to operate on.
 * @param[out] event_code  Filled with the event code.
 * @return ::IOLINK_MASTER_STATUS_OK when complete, ::IOLINK_MASTER_STATUS_PENDING
 *         while active, ::IOLINK_MASTER_ERR_INVALID_ARG, or an ISDU error.
 */
int iolink_master_read_event_code(iolink_master_port_t* port, uint16_t* event_code);
/**
 * @brief Acknowledge the device's pending event.
 *
 * @param port             Port to operate on.
 * @param[out] event_code  Filled with the acknowledged event code.
 * @return ::IOLINK_MASTER_STATUS_OK when the device's event-code read completes;
 *         this read is the explicit ack policy.
 */
int iolink_master_ack_event(iolink_master_port_t* port, uint16_t* event_code);
/**
 * @brief Read and decode the device's detailed event entries.
 *
 * @param port            Port to operate on.
 * @param[out] events     Destination array for decoded events.
 * @param max_events      Capacity of @p events.
 * @param[out] out_count  Filled with the number of events decoded.
 * @return ::IOLINK_MASTER_STATUS_OK when complete, ::IOLINK_MASTER_STATUS_PENDING
 *         while active, ::IOLINK_MASTER_ERR_INVALID_ARG,
 *         ::IOLINK_MASTER_ERR_BUFFER_TOO_SMALL, or an ISDU error.
 */
int iolink_master_read_event_details(iolink_master_port_t* port, iolink_master_event_t* events,
                                     uint8_t max_events, uint8_t* out_count);

/** @} */ /* end of Event services */

/** @name Parameter server (block up/download)
 *  @{
 */

/**
 * @brief Begin a parameter block download to the device.
 *
 * @param port  Port to operate on.
 * @return ::IOLINK_MASTER_STATUS_OK when complete, ::IOLINK_MASTER_STATUS_PENDING
 *         while active, ::IOLINK_MASTER_ERR_INVALID_ARG, or an ISDU error.
 */
int iolink_master_begin_parameter_download(iolink_master_port_t* port);
/**
 * @brief End a parameter block download to the device.
 *
 * @param port  Port to operate on.
 * @return ::IOLINK_MASTER_STATUS_OK when complete, ::IOLINK_MASTER_STATUS_PENDING
 *         while active, ::IOLINK_MASTER_ERR_INVALID_ARG, or an ISDU error.
 */
int iolink_master_end_parameter_download(iolink_master_port_t* port);
/**
 * @brief Begin a parameter block upload from the device.
 *
 * @param port  Port to operate on.
 * @return ::IOLINK_MASTER_STATUS_OK when complete, ::IOLINK_MASTER_STATUS_PENDING
 *         while active, ::IOLINK_MASTER_ERR_INVALID_ARG, or an ISDU error.
 */
int iolink_master_begin_parameter_upload(iolink_master_port_t* port);
/**
 * @brief End a parameter block upload from the device.
 *
 * @param port  Port to operate on.
 * @return ::IOLINK_MASTER_STATUS_OK when complete, ::IOLINK_MASTER_STATUS_PENDING
 *         while active, ::IOLINK_MASTER_ERR_INVALID_ARG, or an ISDU error.
 */
int iolink_master_end_parameter_upload(iolink_master_port_t* port);
/**
 * @brief Store (commit) a downloaded parameter block on the device.
 *
 * @param port  Port to operate on.
 * @return ::IOLINK_MASTER_STATUS_OK when complete, ::IOLINK_MASTER_STATUS_PENDING
 *         while active, ::IOLINK_MASTER_ERR_INVALID_ARG, or an ISDU error.
 */
int iolink_master_store_parameter_download(iolink_master_port_t* port);
/**
 * @brief Write a single parameter block with readback verification.
 *
 * @param port      Port to operate on.
 * @param index     ISDU index.
 * @param subindex  ISDU subindex.
 * @param data      Parameter payload to write.
 * @param len       Number of bytes in @p data.
 * @return ::IOLINK_MASTER_STATUS_OK after download start, write, download end,
 *         and readback verification complete.
 */
int iolink_master_write_parameter_block(iolink_master_port_t* port, uint16_t index,
                                        uint8_t subindex, const uint8_t* data, uint8_t len);

/** @} */ /* end of Parameter server (block up/download) */

/** @name Multi-port controller
 *  @{
 */

/**
 * @brief Initialize a multi-port controller.
 *
 * Initializes each port from the matching `phys[i]`/`configs[i]`. Same lifetime
 * contract as ::iolink_master_init: every PHY is retained by pointer, so the
 * `phys` array (and the PHYs it points to) must outlive the controller. The
 * `ports` array must too.
 *
 * @param controller  Caller-owned opaque controller storage to initialize.
 * @param ports       Array of @p port_count caller-owned port storages.
 * @param port_count  Number of ports.
 * @param phys        Array of PHY APIs, retained by pointer.
 * @param configs     Array of per-port configurations.
 * @return ::IOLINK_MASTER_STATUS_OK, ::IOLINK_MASTER_ERR_INVALID_ARG, or the
 *         first per-port init error.
 */
int iolink_master_controller_init(iolink_master_controller_t* controller,
                                  iolink_master_port_t* ports, uint8_t port_count,
                                  const iolink_phy_api_t* phys,
                                  const iolink_master_config_t* configs);
/**
 * @brief Get the number of ports managed by a controller.
 *
 * @param controller      Controller to query.
 * @param[out] out_count  Filled with the port count.
 * @return ::IOLINK_MASTER_STATUS_OK or ::IOLINK_MASTER_ERR_INVALID_ARG.
 */
int iolink_master_controller_get_port_count(const iolink_master_controller_t* controller,
                                            uint8_t* out_count);
/**
 * @brief Get a pointer to a managed port by index.
 *
 * @param controller     Controller to query.
 * @param index          Zero-based port index.
 * @param[out] out_port  Filled with the addressed port pointer.
 * @return ::IOLINK_MASTER_STATUS_OK or ::IOLINK_MASTER_ERR_INVALID_ARG for
 *         NULL/out-of-range access.
 */
int iolink_master_controller_get_port(iolink_master_controller_t* controller, uint8_t index,
                                      iolink_master_port_t** out_port);
/**
 * @brief Tick every managed port, one response-timeout flag per port.
 *
 * @param controller         Controller to tick.
 * @param response_timeouts  Array of per-port response-timeout flags.
 * @return ::IOLINK_MASTER_STATUS_OK, ::IOLINK_MASTER_ERR_INVALID_ARG, or the
 *         first negative per-port tick result.
 */
int iolink_master_controller_tick(iolink_master_controller_t* controller,
                                  const bool* response_timeouts);
/**
 * @brief Tick every managed port with an explicit per-port tick event.
 *
 * @param controller  Controller to tick.
 * @param events      Array of per-port tick events.
 * @return ::IOLINK_MASTER_STATUS_OK, ::IOLINK_MASTER_ERR_INVALID_ARG, or the
 *         first negative per-port tick-event result.
 */
int iolink_master_controller_tick_events(iolink_master_controller_t* controller,
                                         const iolink_master_tick_event_t* events);
/**
 * @brief Tick every managed port with a shared monotonic timestamp.
 *
 * @param controller  Controller to tick.
 * @param now_100us   Current monotonic time, in 100us units.
 * @return ::IOLINK_MASTER_STATUS_OK, ::IOLINK_MASTER_ERR_INVALID_ARG, or the
 *         first negative per-port time-aware tick result.
 */
int iolink_master_controller_tick_at(iolink_master_controller_t* controller, uint32_t now_100us);
/**
 * @brief Compute the next due tick time for a single port.
 *
 * The caller owns the hardware timer; the output is the next due 100us tick.
 *
 * @param port                Port to query.
 * @param now_100us           Current monotonic time, in 100us units.
 * @param[out] out_next_100us Filled with the next due tick time, in 100us units.
 * @return ::IOLINK_MASTER_STATUS_OK or ::IOLINK_MASTER_ERR_INVALID_ARG.
 */
int iolink_master_get_next_tick_time(const iolink_master_port_t* port, uint32_t now_100us,
                                     uint32_t* out_next_100us);
/**
 * @brief Compute the earliest next due tick time across all ports.
 *
 * @param controller          Controller to query.
 * @param now_100us           Current monotonic time, in 100us units.
 * @param[out] out_next_100us Filled with the earliest next due tick, in 100us units.
 * @return ::IOLINK_MASTER_STATUS_OK or ::IOLINK_MASTER_ERR_INVALID_ARG.
 */
int iolink_master_controller_get_next_tick_time(const iolink_master_controller_t* controller,
                                                uint32_t now_100us, uint32_t* out_next_100us);

/** @} */ /* end of Multi-port controller */

/** @} */ /* end of iolinki_master group */

#endif /* IOLINKI_MASTER_MASTER_H */
