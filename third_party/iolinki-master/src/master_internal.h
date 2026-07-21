#ifndef IOLINKI_MASTER_INTERNAL_H
#define IOLINKI_MASTER_INTERNAL_H

#include "iolinki_master/master.h"

/*
 * Named constants for the master stack. These are master-owned (they intentionally
 * do not modify the shared device-stack protocol.h); values that already have a
 * name in iolinki/protocol.h are reused rather than redefined here.
 */

/* RX/TX scratch buffer size; must hold the worst-case operate frame. */
#define IOLINK_MASTER_FRAME_BUF_SIZE 64U
/* Checksum/response retry budget before entering the error state. */
#define IOLINK_MASTER_RX_RETRY_LIMIT 2U
/* Wake-up request pattern (alternating bits) emitted when no wake_up hook is set. */
#define IOLINK_MASTER_WAKEUP_BYTE 0x55U

/* Direct Parameter Page 1 wire layout (see IO-Link spec Table B.1). */
#define IOLINK_MASTER_DPP1_LEN 16U
#define IOLINK_MASTER_DPP1_OFF_MASTER_COMMAND 0x00U
#define IOLINK_MASTER_DPP1_OFF_MIN_CYCLE_TIME 0x02U
#define IOLINK_MASTER_DPP1_OFF_MSEQ_CAPABILITY 0x03U
#define IOLINK_MASTER_DPP1_OFF_REVISION_ID 0x04U
#define IOLINK_MASTER_DPP1_OFF_PD_IN_DESC 0x05U
#define IOLINK_MASTER_DPP1_OFF_PD_OUT_DESC 0x06U
#define IOLINK_MASTER_DPP1_OFF_VENDOR_ID_HI 0x07U
#define IOLINK_MASTER_DPP1_OFF_VENDOR_ID_LO 0x08U
#define IOLINK_MASTER_DPP1_OFF_DEVICE_ID_HI 0x09U
#define IOLINK_MASTER_DPP1_OFF_DEVICE_ID_MID 0x0AU
#define IOLINK_MASTER_DPP1_OFF_DEVICE_ID_LO 0x0BU

/* IO-Link protocol revision IDs (RevisionID octet, Figure B.4). */
#define IOLINK_MASTER_REVISION_1_0 0x10U
#define IOLINK_MASTER_REVISION_1_1 0x11U

/* MinCycleTime octet fields (Figure B.2 / Table B.3). */
#define IOLINK_MASTER_MIN_CYCLE_BASE_SHIFT 6U
#define IOLINK_MASTER_MIN_CYCLE_BASE_MASK 0x03U
#define IOLINK_MASTER_MIN_CYCLE_MULT_MASK 0x3FU

/* M-sequenceCapability octet bit fields (Figure B.3). */
#define IOLINK_MASTER_MSEQ_CAP_ISDU_BIT 0x01U
#define IOLINK_MASTER_MSEQ_CAP_OPERATE_SHIFT 1U
#define IOLINK_MASTER_MSEQ_CAP_OPERATE_MASK 0x07U
#define IOLINK_MASTER_MSEQ_CAP_PREOP_SHIFT 4U
#define IOLINK_MASTER_MSEQ_CAP_PREOP_MASK 0x03U

/* ProcessData descriptor octet fields (Figure B.5 / Table B.6). */
#define IOLINK_MASTER_PD_DESC_BYTE_BIT 0x80U
#define IOLINK_MASTER_PD_DESC_LENGTH_MASK 0x1FU
#define IOLINK_MASTER_PD_DESC_BITS_PER_OCTET 8U

/* Master Command comm-channel field position (pairs with IOLINK_MC_COMM_CHANNEL_MASK). */
#define IOLINK_MASTER_MC_COMM_CHANNEL_SHIFT 5U

/* ISDU framing. */
#define IOLINK_MASTER_ISDU_SERVICE_SHIFT 4U
#define IOLINK_MASTER_ISDU_RESPONSE_ERROR 0x80U
#define IOLINK_MASTER_ISDU_LENGTH_NIBBLE_MAX 15U
#define IOLINK_MASTER_ISDU_LENGTH_EXTENDED 0x0FU
#define IOLINK_MASTER_ISDU_WRITE_HEADER_MAX 5U
#define IOLINK_MASTER_ISDU_READ_HEADER_LEN 4U

/* Data Storage record header + event-entry framing. */
#define IOLINK_MASTER_DS_RECORD_HEADER_LEN 4U
#define IOLINK_MASTER_EVENT_ENTRY_LEN 3U
#define IOLINK_MASTER_MAX_EVENTS 8U
#define IOLINK_MASTER_EVENT_QUALIFIER_MODE_SHIFT 4U
#define IOLINK_MASTER_EVENT_QUALIFIER_MODE_MASK 0x03U
#define IOLINK_MASTER_EVENT_MODE_NOTIFICATION 1U
#define IOLINK_MASTER_EVENT_MODE_WARNING 2U
#define IOLINK_MASTER_EVENT_MODE_ERROR 3U

/* Startup micro-sequence steps (state of iolink_master_startup_state_t.step). */
#define IOLINK_MASTER_STARTUP_STEP_WAKE 0U
#define IOLINK_MASTER_STARTUP_STEP_SEND_TYPE0 1U
#define IOLINK_MASTER_STARTUP_STEP_AWAIT_RESPONSE 2U

typedef struct
{
    uint8_t step;
    uint8_t baudrate_index;
    uint8_t wake_attempts;
} iolink_master_startup_state_t;

typedef struct
{
    iolink_master_isdu_op_t op;
    uint16_t index;
    uint8_t subindex;
    uint8_t request[IOLINK_ISDU_BUFFER_SIZE];
    uint8_t request_len;
    uint8_t request_pos;
    uint8_t request_seq;
    bool request_control_phase;
    bool request_sent;
    uint8_t response[IOLINK_ISDU_BUFFER_SIZE];
    uint16_t response_len;
    uint8_t response_seq;
    bool response_expect_control;
    bool response_last;
    bool done;
    uint8_t error;
} iolink_master_isdu_state_t;

typedef struct
{
    uint8_t buf[IOLINK_MASTER_FRAME_BUF_SIZE];
    uint8_t len;
} iolink_master_rx_state_t;

typedef enum
{
    IOLINK_MASTER_BLOCK_STEP_NONE = 0,
    IOLINK_MASTER_BLOCK_STEP_BEGIN_DOWNLOAD = 1,
    IOLINK_MASTER_BLOCK_STEP_WRITE = 2,
    IOLINK_MASTER_BLOCK_STEP_END_DOWNLOAD = 3,
    IOLINK_MASTER_BLOCK_STEP_VERIFY = 4
} iolink_master_block_step_t;

typedef struct
{
    iolink_master_block_step_t step;
    uint16_t index;
    uint8_t subindex;
    uint8_t data[IOLINK_ISDU_BUFFER_SIZE];
    uint8_t len;
} iolink_master_block_state_t;

typedef struct
{
    const iolink_phy_api_t* phy;
    iolink_master_config_t config;
    iolink_master_state_t state;
    uint8_t od_len;
    uint8_t tx_buf[IOLINK_MASTER_FRAME_BUF_SIZE];
    uint8_t pd_in[IOLINK_PD_IN_MAX_SIZE];
    uint8_t pd_in_len;
    uint8_t pd_out[IOLINK_PD_OUT_MAX_SIZE];
    uint8_t pd_out_len;
    bool pd_valid;
    iolink_master_startup_state_t startup;
    iolink_master_diagnostics_t diagnostics;
    iolink_master_device_info_t device_info;
    iolink_master_isdu_state_t isdu;
    iolink_master_block_state_t block;
    iolink_master_rx_state_t rx;
    uint32_t cycle_count;
    uint32_t last_cycle_start_100us;
    uint32_t response_deadline_100us;
    bool cycle_timer_valid;
    bool awaiting_response;
} iolink_master_port_state_t;

typedef struct
{
    iolink_master_port_t* ports;
    uint8_t port_count;
} iolink_master_controller_state_t;

typedef char iolink_master_port_storage_must_fit
    [(sizeof(iolink_master_port_state_t) <= IOLINK_MASTER_PORT_STORAGE_SIZE) ? 1 : -1];
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
static inline iolink_master_port_state_t* iolink_master_port_state(iolink_master_port_t* port)
{
    return (iolink_master_port_state_t*)(void*)port->storage;
}

static inline const iolink_master_port_state_t*
iolink_master_port_const_state(const iolink_master_port_t* port)
{
    return (const iolink_master_port_state_t*)(const void*)port->storage;
}

static inline iolink_master_controller_state_t*
iolink_master_controller_state(iolink_master_controller_t* controller)
{
    return (iolink_master_controller_state_t*)(void*)controller->storage;
}

static inline const iolink_master_controller_state_t*
iolink_master_controller_const_state(const iolink_master_controller_t* controller)
{
    return (const iolink_master_controller_state_t*)(const void*)controller->storage;
}

static inline uint8_t iolink_master_od_len_for_type(iolink_master_m_seq_type_t type)
{
    switch(type)
    {
    case IOLINK_MASTER_M_SEQ_TYPE_2_1:
    case IOLINK_MASTER_M_SEQ_TYPE_2_2:
    case IOLINK_MASTER_M_SEQ_TYPE_2_V:
        return 2U;
    default:
        return 1U;
    }
}

static inline bool iolink_master_response_due_at(const iolink_master_port_t* port,
                                                 uint32_t now_100us)
{
    const iolink_master_port_state_t* state = iolink_master_port_const_state(port);

    return state->awaiting_response && (now_100us >= state->response_deadline_100us);
}

void iolink_master_isdu_fill_od(iolink_master_port_t* port, uint8_t* od, uint8_t od_len);
void iolink_master_isdu_on_od(iolink_master_port_t* port, const uint8_t* od, uint8_t od_len);

#endif /* IOLINKI_MASTER_INTERNAL_H */
