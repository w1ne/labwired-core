/**
 * @file master_internal.h
 * @brief Private internal state and named constants for the IO-Link master stack.
 *
 * This header is not part of the public ABI. It defines the master-owned named
 * constants (wire offsets, framing budgets, encoding masks), the private
 * per-port and controller state structures backed by the public opaque
 * storage, the static-assert size guards, and the inline accessors that
 * reinterpret the caller-owned opaque storage as the private state.
 */
#ifndef IOLINKI_MASTER_INTERNAL_H
#define IOLINKI_MASTER_INTERNAL_H

#include "iolinki_master/master.h"

/**
 * @defgroup iolinki_master_internal Master Internal State
 * @brief Private internal state, constants and accessors for the master stack.
 * @{
 */

/*
 * Named constants for the master stack. These are master-owned (they intentionally
 * do not modify the shared device-stack protocol.h); values that already have a
 * name in iolinki/protocol.h are reused rather than redefined here.
 */

/** @brief RX/TX scratch buffer size; must hold the worst-case operate frame. */
#define IOLINK_MASTER_FRAME_BUF_SIZE 64U
/** @brief Checksum/response retry budget before restarting communication (7.2.2.1). */
#define IOLINK_MASTER_RX_RETRY_LIMIT 2U

/** @name Timing defaults and conversion (Table 9, Table 42, A.3.5/A.3.6).
 *  @{
 */
#define IOLINK_MASTER_T_BIT_COM1_NS 208330U /**< T_BIT at COM1 (208.33 us). */
#define IOLINK_MASTER_T_BIT_COM2_NS 26040U  /**< T_BIT at COM2 (26.04 us). */
#define IOLINK_MASTER_T_BIT_COM3_NS 4340U   /**< T_BIT at COM3 (4.34 us). */
#define IOLINK_MASTER_NS_PER_100US 100000U  /**< Nanoseconds per 100us tick. */
#define IOLINK_MASTER_DEFAULT_T_DMT_TBIT 32U /**< Default T_DMT in bit times (Table 42). */
#define IOLINK_MASTER_DEFAULT_T_DWU_100US 400U /**< Default T_DWU, 40 ms (Table 42). */
#define IOLINK_MASTER_DEFAULT_WAKE_RETRY_LIMIT 2U /**< Default n_WU (Table 42). */
/** @brief Default T_SD, 500 ms: spacing of wake-up retry sequences (Table 42, 0.5..1 s). */
#define IOLINK_MASTER_DEFAULT_T_SD_100US 5000U
/** @brief T_REN maximum, 500 us, in 100us ticks (Table 10). */
#define IOLINK_MASTER_T_REN_MAX_100US 5U
/**
 * @brief Worst-case TYPE_0 read M-sequence time in bit times (A.3.6, equation A.6).
 *
 * m = 2 master frames (MC, CKT), n = 2 device frames (OD, CKS):
 * (2 + 2) * 11 + t_A (10, A.5) + (2 - 1) * t1 (1, A.3) + (2 - 1) * t2 (3, A.4) = 58.
 */
#define IOLINK_MASTER_TYPE0_M_SEQ_MAX_TBIT 58U
/** @brief UART frame length in bit times (1 start + 8 data + 1 parity + 1 stop). */
#define IOLINK_MASTER_UART_FRAME_TBIT 11U
/** @brief Maximum device response delay in bit times (t_A, A.3.5). */
#define IOLINK_MASTER_T_A_MAX_TBIT 10U
/** @} */
/** @brief Wake-up request pattern (alternating bits) emitted when no wake_up hook is set. */
#define IOLINK_MASTER_WAKEUP_BYTE 0x55U

/** @name Direct Parameter Page 1 wire layout (see IO-Link spec Table B.1).
 *  @{
 */
#define IOLINK_MASTER_DPP1_LEN 16U /**< Direct Parameter Page 1 length, in bytes. */
#define IOLINK_MASTER_DPP1_OFF_MASTER_COMMAND 0x00U /**< Offset of the MasterCommand octet. */
#define IOLINK_MASTER_DPP1_OFF_MIN_CYCLE_TIME 0x02U /**< Offset of the MinCycleTime octet. */
#define IOLINK_MASTER_DPP1_OFF_MSEQ_CAPABILITY \
    0x03U                                          /**< Offset of the M-sequenceCapability octet. */
#define IOLINK_MASTER_DPP1_OFF_REVISION_ID 0x04U   /**< Offset of the RevisionID octet. */
#define IOLINK_MASTER_DPP1_OFF_PD_IN_DESC 0x05U    /**< Offset of the input PD descriptor octet. */
#define IOLINK_MASTER_DPP1_OFF_PD_OUT_DESC 0x06U   /**< Offset of the output PD descriptor octet. */
#define IOLINK_MASTER_DPP1_OFF_VENDOR_ID_HI 0x07U  /**< Offset of the VendorID high octet. */
#define IOLINK_MASTER_DPP1_OFF_VENDOR_ID_LO 0x08U  /**< Offset of the VendorID low octet. */
#define IOLINK_MASTER_DPP1_OFF_DEVICE_ID_HI 0x09U  /**< Offset of the DeviceID high octet. */
#define IOLINK_MASTER_DPP1_OFF_DEVICE_ID_MID 0x0AU /**< Offset of the DeviceID middle octet. */
#define IOLINK_MASTER_DPP1_OFF_DEVICE_ID_LO 0x0BU  /**< Offset of the DeviceID low octet. */
/** @} */

/** @name IO-Link protocol revision IDs (RevisionID octet, Figure B.4).
 *  @{
 */
#define IOLINK_MASTER_REVISION_1_0 0x10U /**< RevisionID for IO-Link 1.0. */
#define IOLINK_MASTER_REVISION_1_1 0x11U /**< RevisionID for IO-Link 1.1. */
/** @} */

/** @name MinCycleTime octet fields (Figure B.2 / Table B.3).
 *  @{
 */
#define IOLINK_MASTER_MIN_CYCLE_BASE_SHIFT 6U   /**< Shift of the time-base field. */
#define IOLINK_MASTER_MIN_CYCLE_BASE_MASK 0x03U /**< Mask of the time-base field (post-shift). */
#define IOLINK_MASTER_MIN_CYCLE_MULT_MASK 0x3FU /**< Mask of the multiplier field. */
/** @} */

/** @name M-sequenceCapability octet bit fields (Figure B.3).
 *  @{
 */
#define IOLINK_MASTER_MSEQ_CAP_ISDU_BIT 0x01U     /**< ISDU-supported bit. */
#define IOLINK_MASTER_MSEQ_CAP_OPERATE_SHIFT 1U   /**< OPERATE M-seq code field shift. */
#define IOLINK_MASTER_MSEQ_CAP_OPERATE_MASK 0x07U /**< OPERATE M-seq code field mask. */
#define IOLINK_MASTER_MSEQ_CAP_PREOP_SHIFT 4U     /**< PREOPERATE M-seq code field shift. */
#define IOLINK_MASTER_MSEQ_CAP_PREOP_MASK \
    0x03U /**< Mask of the PREOPERATE M-sequence code field. */
/** @} */

/** @name ProcessData descriptor octet fields (Figure B.5 / Table B.6).
 *  @{
 */
#define IOLINK_MASTER_PD_DESC_BYTE_BIT 0x80U    /**< Byte/bit length-unit selector bit. */
#define IOLINK_MASTER_PD_DESC_LENGTH_MASK 0x1FU /**< Mask of the PD length field. */
#define IOLINK_MASTER_PD_DESC_BITS_PER_OCTET 8U /**< Bits per octet for bit-length conversion. */
/** @} */

/** @brief Master Command comm-channel field position (pairs with IOLINK_MC_COMM_CHANNEL_MASK). */
#define IOLINK_MASTER_MC_COMM_CHANNEL_SHIFT 5U

/** @name Reply checksum octet (CKS) flags (A.1.5).
 *  @{
 */
#define IOLINK_MASTER_CKS_EVENT 0x80U     /**< Bit 7: an event is pending (freeze until acked). */
#define IOLINK_MASTER_CKS_PD_INVALID 0x40U /**< Bit 6: PD-in is invalid (set = invalid). */
#define IOLINK_MASTER_CKS_CHECKSUM_MASK 0x3FU /**< Bits 0-5: the A.1.6 message checksum. */
/** @} */

/** @name ISDU framing (7.3.6.1, A.5).
 *  @{
 */
#define IOLINK_MASTER_ISDU_SERVICE_SHIFT 4U     /**< Shift of the ISDU service nibble. */
#define IOLINK_MASTER_ISDU_RESPONSE_ERROR 0x80U /**< Bit marking an ISDU error response. */
#define IOLINK_MASTER_ISDU_LENGTH_NIBBLE_MAX \
    15U /**< Max length representable in the length nibble. */
#define IOLINK_MASTER_ISDU_LENGTH_EXTENDED \
    0x01U /**< Length nibble value selecting ExtLength (Table A.14). */
#define IOLINK_MASTER_ISDU_EXT_MIN 17U /**< Minimum total length using ExtLength (A.5.3). */
#define IOLINK_MASTER_ISDU_EXT_MAX 238U /**< Maximum total length using ExtLength (A.5.3). */
#define IOLINK_MASTER_ISDU_WRITE_HEADER_MAX 7U /**< Max ISDU write header incl. ExtLength+CHKPDU. */
#define IOLINK_MASTER_ISDU_READ_HEADER_MAX 6U  /**< Max ISDU read header incl. ExtLength+CHKPDU. */
#define IOLINK_MASTER_ISDU_CHKPDU_LEN 1U       /**< CHKPDU octet length (A.5.6). */
/** @} */

/** @name Data Storage record header + event-entry framing.
 *  @{
 */
#define IOLINK_MASTER_DS_RECORD_HEADER_LEN 4U /**< Data Storage record header length, in bytes. */
#define IOLINK_MASTER_EVENT_ENTRY_LEN 3U      /**< Length of one event entry, in bytes. */
#define IOLINK_MASTER_MAX_EVENTS 8U           /**< Maximum events decoded per read. */
#define IOLINK_MASTER_EVENT_MEMORY_LEN 19U    /**< Table 58 event memory size (0x00-0x12). */
#define IOLINK_MASTER_EVENT_SLOT_MAX 6U       /**< Highest Table 58 event slot. */
#define IOLINK_MASTER_EVENT_STATUS_DETAILS 0x80U /**< StatusCode bit 7: details present. */
#define IOLINK_MASTER_EVENT_QUALIFIER_MODE_SHIFT 4U /**< Event-qualifier mode field shift. */
#define IOLINK_MASTER_EVENT_QUALIFIER_MODE_MASK \
    0x03U                                        /**< Mask of the event-qualifier mode field. */
#define IOLINK_MASTER_EVENT_MODE_NOTIFICATION 1U /**< Qualifier mode: notification. */
#define IOLINK_MASTER_EVENT_MODE_WARNING 2U      /**< Qualifier mode: warning. */
#define IOLINK_MASTER_EVENT_MODE_ERROR 3U        /**< Qualifier mode: error. */
/** @} */

/** @name Startup micro-sequence steps (state of iolink_master_startup_state_t.step).
 *  @{
 */
#define IOLINK_MASTER_STARTUP_STEP_WAKE 0U           /**< Emit the wake-up request. */
#define IOLINK_MASTER_STARTUP_STEP_SEND_TYPE0 1U     /**< Send the TYPE_0 request. */
#define IOLINK_MASTER_STARTUP_STEP_AWAIT_RESPONSE 2U /**< Await the device response. */
#define IOLINK_MASTER_STARTUP_STEP_AWAIT_OPERATE_ACK 3U /**< Await the CKS reply to DeviceOperate. */
/** Wake-up retry sequence failed: PHY inactive, waiting T_SD before the next one (7.3.2.2). */
#define IOLINK_MASTER_STARTUP_STEP_INACTIVE 4U
/** @} */

/** @brief Startup micro-sequence progress for a port. */
typedef struct
{
    uint8_t step;           /**< Current startup step (IOLINK_MASTER_STARTUP_STEP_*). */
    uint8_t baudrate_index; /**< Index into the COM3, COM2, COM1 sweep (auto-baud). */
    uint8_t wake_attempts;  /**< Retry counter of Figure 36: failed WURQ + rate scans so far. */
} iolink_master_startup_state_t;

/** @brief Transport phase of the master ISDU handler (7.3.6.3, Figure 51). */
typedef enum
{
    IOLINK_MASTER_ISDU_PHASE_NONE = 0, /**< Idle: no ISDU service in progress. */
    IOLINK_MASTER_ISDU_PHASE_REQUEST,  /**< Sending the request octet stream (T2/T3). */
    IOLINK_MASTER_ISDU_PHASE_WAIT,     /**< Polling until the response starts (T5). */
    IOLINK_MASTER_ISDU_PHASE_RESPONSE, /**< Receiving response octets (T7). */
} iolink_master_isdu_phase_t;

/** @brief In-flight ISDU request/response state machine for a port.
 *
 * The ISDU octet stream (7.3.6.1, A.5) is assembled in @c request or received
 * into @c response; it is segmented over M-sequences on the ISDU channel with
 * the FlowCTRL counter carried in the MC address bits (A.1.2, Table 52).
 */
typedef struct
{
    iolink_master_isdu_op_t op;                /**< Current ISDU operation kind. */
    iolink_master_isdu_phase_t phase;          /**< Current transport phase. */
    uint16_t index;                            /**< ISDU index being accessed. */
    uint8_t subindex;                          /**< ISDU subindex being accessed. */
    uint8_t request[IOLINK_ISDU_BUFFER_SIZE];  /**< Assembled ISDU request buffer. */
    uint16_t request_len;                      /**< Total request length, in bytes. */
    uint16_t request_pos;                      /**< Bytes of the request already sent. */
    uint8_t flowctrl;                          /**< Next outgoing FlowCTRL (Table 52). */
    uint8_t chk;                               /**< Running CHKPDU accumulator (A.5.6). */
    uint16_t expected_len;                     /**< Declared ISDU stream length, in bytes. */
    uint8_t response[IOLINK_ISDU_BUFFER_SIZE]; /**< Assembled ISDU response buffer. */
    uint16_t response_len;                     /**< Total response length, in bytes. */
    uint16_t response_pos;                     /**< Bytes of the response already decoded. */
    bool done;          /**< True once the operation has completed. */
    bool idle_pending;  /**< True when the T8 FlowCTRL IDLE read must be sent. */
    bool abort_pending; /**< True when an ABORT M-sequence must be sent (Table 52/53 T11). */
    uint16_t error;     /**< ISDU ErrorType (ErrorCode<<8|AdditionalCode). */
} iolink_master_isdu_state_t;

/** @brief Transport phase of the master Event handler (7.3.8.3, Figure 55). */
typedef enum
{
    IOLINK_MASTER_EVENT_PHASE_NONE = 0, /**< Idle: no event service in progress. */
    IOLINK_MASTER_EVENT_PHASE_READ,     /**< Reading the event memory (T2/T3). */
    IOLINK_MASTER_EVENT_PHASE_WRITE,    /**< Confirming StatusCode (T8). */
} iolink_master_event_phase_t;

/** @brief Kind of event service requested through the diagnosis channel. */
typedef enum
{
    IOLINK_MASTER_EVENT_REQ_NONE = 0, /**< No request. */
    IOLINK_MASTER_EVENT_REQ_CODE,     /**< Report the first active event code. */
    IOLINK_MASTER_EVENT_REQ_DETAILS,  /**< Read and deliver all active events. */
    IOLINK_MASTER_EVENT_REQ_ACK,      /**< Read active events, then confirm. */
} iolink_master_event_req_t;

/** @brief Event-handler state: Table 58 event memory readout over DIAGNOSIS.
 *
 * The master reads the event memory octet by octet (7.3.8.2/Table 59) and, when
 * the request asks for confirmation, writes any value to the StatusCode at
 * address 0 to release the device's frozen event memory (Table 59 T8).
 */
typedef struct
{
    iolink_master_event_phase_t phase; /**< Current transport phase. */
    iolink_master_event_req_t request; /**< Active service kind. */
    uint8_t addr;                      /**< Next event-memory address to read. */
    uint8_t needed;                    /**< Total memory octets to collect. */
    uint8_t memory[IOLINK_MASTER_EVENT_MEMORY_LEN]; /**< Collected memory image. */
    uint8_t len;                       /**< Octets collected so far. */
    uint8_t last_slot;                 /**< Highest active event slot (1..6). */
    bool status_seen;                  /**< True once the StatusCode is decoded. */
    /** True from a diagnosis READ send until its reply OD is consumed: only the
     *  reply to a diagnosis read may be routed into the event memory. A cyclic
     *  reply still in flight when the readout starts must not complete it. */
    bool od_expected;
    int result;                        /**< Latched final result. */
} iolink_master_event_state_t;

/** @brief Receive assembly buffer for a port. */
typedef struct
{
    uint8_t buf[IOLINK_MASTER_FRAME_BUF_SIZE]; /**< Accumulated receive bytes. */
    uint8_t len;                               /**< Number of valid bytes in @c buf. */
} iolink_master_rx_state_t;

/** @brief Step of a block up/download parameter-server operation. */
typedef enum
{
    IOLINK_MASTER_BLOCK_STEP_NONE = 0,           /**< No block operation active. */
    IOLINK_MASTER_BLOCK_STEP_BEGIN_DOWNLOAD = 1, /**< Begin-download command pending/active. */
    IOLINK_MASTER_BLOCK_STEP_WRITE = 2,          /**< Payload write pending/active. */
    IOLINK_MASTER_BLOCK_STEP_END_DOWNLOAD = 3,   /**< End-download command pending/active. */
    IOLINK_MASTER_BLOCK_STEP_VERIFY = 4          /**< Readback verification pending/active. */
} iolink_master_block_step_t;

/** @brief State for a multi-step parameter block write. */
typedef struct
{
    iolink_master_block_step_t step;       /**< Current block-operation step. */
    uint16_t index;                        /**< Target ISDU index. */
    uint8_t subindex;                      /**< Target ISDU subindex. */
    uint8_t data[IOLINK_ISDU_BUFFER_SIZE]; /**< Payload to write / verify. */
    uint8_t len;                           /**< Payload length, in bytes. */
} iolink_master_block_state_t;

/** @brief Private per-port state backed by ::iolink_master_port_t storage. */
typedef struct
{
    const iolink_phy_api_t* phy;                  /**< PHY API, retained by pointer (not copied). */
    iolink_master_config_t config;                /**< Copy of the port configuration. */
    iolink_master_state_t state;                  /**< Current communication state. */
    uint8_t od_len;                               /**< On-request-data length for the M-sequence. */
    uint8_t tx_buf[IOLINK_MASTER_FRAME_BUF_SIZE]; /**< Transmit scratch buffer. */
    uint8_t pd_in[IOLINK_PD_IN_MAX_SIZE];         /**< Latest input process data. */
    uint8_t pd_in_len;                            /**< Input process-data length, in bytes. */
    uint8_t pd_out[IOLINK_PD_OUT_MAX_SIZE];       /**< Staged output process data. */
    uint8_t pd_out_len;                           /**< Output process-data length, in bytes. */
    bool pd_valid;                                /**< True when @c pd_in holds valid data. */
    iolink_master_startup_state_t startup;        /**< Startup micro-sequence state. */
    iolink_master_diagnostics_t diagnostics;      /**< Runtime diagnostics counters. */
    iolink_master_device_info_t device_info;      /**< Decoded device identification. */
    iolink_master_isdu_state_t isdu;              /**< In-flight ISDU state machine. */
    iolink_master_event_state_t event;            /**< Event-memory readout state machine. */
    iolink_master_block_state_t block;            /**< Parameter block operation state. */
    iolink_master_rx_state_t rx;                  /**< Receive assembly buffer. */
    uint32_t cycle_count;                         /**< Number of cycles executed. */
    uint32_t last_cycle_start_100us;  /**< Last cycle start timestamp, in 100us units. */
    uint32_t response_deadline_100us; /**< Response deadline timestamp, in 100us units. */
    uint32_t send_ready_at_100us;     /**< Earliest transmit timestamp, in 100us units. */
    bool send_ready_valid;            /**< True when @c send_ready_at_100us gates transmits. */
    bool cycle_timer_valid;           /**< True once the cycle timer has a valid start. */
    bool awaiting_response;           /**< True while waiting on a device response. */
    /** True when the reply to the outstanding frame carries OD octets: a READ
     *  M-sequence is answered with [OD][PD-in] CKS, a WRITE with [PD-in] CKS
     *  (Figures A.9/A.10/A.13/A.14). */
    bool rx_od_expected;
} iolink_master_port_state_t;

/** @brief Private controller state backed by ::iolink_master_controller_t storage. */
typedef struct
{
    iolink_master_port_t* ports; /**< Caller-owned port array reference. */
    uint8_t port_count;          /**< Number of managed ports. */
} iolink_master_controller_state_t;

/** @brief Compile-time assert that the port state fits its opaque storage budget. */
typedef char iolink_master_port_storage_must_fit
    [(sizeof(iolink_master_port_state_t) <= IOLINK_MASTER_PORT_STORAGE_SIZE) ? 1 : -1];
/** @brief Compile-time assert that the controller state fits its opaque storage budget. */
typedef char iolink_master_controller_storage_must_fit
    [(sizeof(iolink_master_controller_state_t) <= IOLINK_MASTER_CONTROLLER_STORAGE_SIZE) ? 1 : -1];

/*
 * These accessors reinterpret the caller-owned opaque storage as the private
 * state struct. The `void*` cast is a deliberate, documented deviation from
 * MISRA C:2012 Rule 11.5: the public ABI keeps the state opaque and heap-free,
 * and the `_storage_must_fit` static asserts above guarantee the storage is
 * large enough (and the union alignment members in master.h guarantee
 * alignment). See docs/MISRA_DEVIATIONS.md.
 */

/** @brief Reinterpret opaque port storage as mutable private port state. */
static inline iolink_master_port_state_t* iolink_master_port_state(iolink_master_port_t* port)
{
    return (iolink_master_port_state_t*) (void*) port->storage;
}

/** @brief Reinterpret opaque port storage as const private port state. */
static inline const iolink_master_port_state_t* iolink_master_port_const_state(
    const iolink_master_port_t* port)
{
    return (const iolink_master_port_state_t*) (const void*) port->storage;
}

/** @brief Reinterpret opaque controller storage as mutable private controller state. */
static inline iolink_master_controller_state_t* iolink_master_controller_state(
    iolink_master_controller_t* controller)
{
    return (iolink_master_controller_state_t*) (void*) controller->storage;
}

/** @brief Reinterpret opaque controller storage as const private controller state. */
static inline const iolink_master_controller_state_t* iolink_master_controller_const_state(
    const iolink_master_controller_t* controller)
{
    return (const iolink_master_controller_state_t*) (const void*) controller->storage;
}

/** @brief Return the A.1.3 CKT M-sequence type field (bits 6-7) for an M-sequence type. */
static inline uint8_t iolink_master_ckt_type_bits(iolink_master_m_seq_type_t type)
{
    switch (type) {
        case IOLINK_MASTER_M_SEQ_TYPE_1_1:
        case IOLINK_MASTER_M_SEQ_TYPE_1_2:
        case IOLINK_MASTER_M_SEQ_TYPE_1_V:
            return 0x40U;
        case IOLINK_MASTER_M_SEQ_TYPE_2_1:
        case IOLINK_MASTER_M_SEQ_TYPE_2_2:
        case IOLINK_MASTER_M_SEQ_TYPE_2_V:
            return 0x80U;
        default:
            return 0x00U;
    }
}

/** @brief Return the on-request-data length in octets for an M-sequence type.
 *
 * Table A.10: TYPE_2_1 to TYPE_2_6 carry one OD octet; only TYPE_2_V is the
 * two-octet variant (OPERATE M-sequence code 5). */
static inline uint8_t iolink_master_od_len_for_type(iolink_master_m_seq_type_t type)
{
    switch (type) {
        case IOLINK_MASTER_M_SEQ_TYPE_2_V:
            return 2U;
        default:
            return 1U;
    }
}

/** @brief Test whether a port's response deadline has elapsed at @p now_100us. */
static inline bool iolink_master_response_due_at(const iolink_master_port_t* port,
                                                 uint32_t now_100us)
{
    const iolink_master_port_state_t* state = iolink_master_port_const_state(port);

    return state->awaiting_response && (now_100us >= state->response_deadline_100us);
}

/** @brief Fill the on-request-data octets of an outgoing frame from ISDU state. */
void iolink_master_isdu_fill_od(iolink_master_port_t* port, uint8_t* od, uint8_t od_len);
/** @brief Consume the on-request-data octets of a received frame into ISDU state. */
void iolink_master_isdu_on_od(iolink_master_port_t* port, const uint8_t* od, uint8_t od_len);

/** @brief Return true while an ISDU service is being transported on the ISDU channel.
 *
 * When true, @p read reports the required M-sequence direction (true = device to
 * master) and @p flowctrl the FlowCTRL value (Table 52) to place in the MC
 * address bits. The caller builds @c MC = read<<7 | IOLINK_MC_CHANNEL_ISDU |
 * flowctrl (A.1.2).
 */
bool iolink_master_isdu_channel_access(const iolink_master_port_t* port, bool* read,
                                       uint8_t* flowctrl);

/** @brief Consume a pending ISDU ABORT request (Table 53 T11).
 *
 * Returns true once per latched transport error, after which the caller must
 * emit an ISDU-channel ABORT M-sequence. The latch is cleared before returning
 * so the caller does not need to touch private state.
 */
bool iolink_master_isdu_take_abort(iolink_master_port_t* port);

/** @brief Consume a pending T8 IDLE request (Table 53).
 *
 * Returns true once after a response has been decoded, after which the caller
 * must conclude the service with an ISDU-channel read carrying FlowCTRL IDLE.
 */
bool iolink_master_isdu_take_idle(iolink_master_port_t* port);

/** @brief Return true while an event-memory service is active on DIAGNOSIS.
 *
 * When true, @p read reports the M-sequence direction (true = device to master,
 * a memory read; false = master write of the StatusCode confirmation), @p addr
 * the event-memory address to place in the MC address bits, and @p od_len the
 * number of OD octets to include (1 for a read of one memory octet).
 */
bool iolink_master_event_channel_access(const iolink_master_port_t* port, bool* read, uint8_t* addr,
                                        uint8_t* od_len);

/** @brief Consume the OD octets of an event-memory read reply (Table 59 T3). */
void iolink_master_event_on_od(iolink_master_port_t* port, const uint8_t* od, uint8_t od_len);

/** @brief Consume the reply to the StatusCode confirmation write (Table 59 T8). */
void iolink_master_event_on_written(iolink_master_port_t* port);

/** @brief Derive the OPERATE M-sequence capability code (Table A.10).
 *
 * @p type is the configured M-sequence type and @p od_len its on-request-data
 * octet count. Returns the OPERATE code (0/1/4/5/6/7) advertised in Direct
 * Parameter Page 1, or 0 for the process-data types where Table A.10 fixes 0.
 */
uint8_t iolink_master_mseq_capability_code(iolink_master_m_seq_type_t type, uint8_t od_len);

/** @brief Run the port state machine applying the timestamped startup timing gates.
 *
 * Like ::iolink_master_process, but @p now_100us gates the T_DMT (Table 42) wait
 * after a wake-up and the T_DWU wake-retry spacing. @p timed selects the gated
 * path; the public ::iolink_master_process calls this with @c timed = false.
 */
void iolink_master_process_at(iolink_master_port_t* port, uint32_t now_100us, bool timed);

/** @brief Apply a response/deadline timeout, honoring the timestamped wake spacing.
 *
 * Like ::iolink_master_on_timeout, but records the T_DWU deadline relative to
 * @p now_100us. @p timed selects the gated path; the public
 * ::iolink_master_on_timeout calls this with @c timed = false.
 */
int iolink_master_on_timeout_at(iolink_master_port_t* port, uint32_t now_100us, bool timed);

/** @} */ /* end of iolinki_master_internal group */

#endif /* IOLINKI_MASTER_INTERNAL_H */
