#ifndef IOLINKI_MASTER_INTERNAL_H
#define IOLINKI_MASTER_INTERNAL_H

#include "iolinki_master/master.h"

typedef struct
{
    uint8_t step;
    uint8_t baudrate_index;
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
    uint8_t buf[64];
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
    uint8_t tx_buf[64];
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
    case IOLINK_MASTER_M_SEQ_TYPE_2_V:
        return 4U;
    case IOLINK_MASTER_M_SEQ_TYPE_2_1:
    case IOLINK_MASTER_M_SEQ_TYPE_2_2:
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
