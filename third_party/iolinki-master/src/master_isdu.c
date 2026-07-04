#include "master_internal.h"

#include "iolinki/protocol.h"

#include <string.h>

static bool iolink_master_isdu_busy(const iolink_master_port_t* port)
{
    const iolink_master_port_state_t* state = iolink_master_port_const_state(port);

    return (state->isdu.op != IOLINK_MASTER_ISDU_OP_NONE) && !state->isdu.done;
}

static bool iolink_master_isdu_matches(const iolink_master_port_t* port,
                                       iolink_master_isdu_op_t op,
                                       uint16_t index,
                                       uint8_t subindex)
{
    const iolink_master_port_state_t* state = iolink_master_port_const_state(port);

    return (state->isdu.op == op) && (state->isdu.index == index) &&
           (state->isdu.subindex == subindex);
}

static void iolink_master_isdu_clear(iolink_master_port_t* port)
{
    iolink_master_port_state(port)->isdu.op = IOLINK_MASTER_ISDU_OP_NONE;
    iolink_master_port_state(port)->isdu.index = 0U;
    iolink_master_port_state(port)->isdu.subindex = 0U;
    iolink_master_port_state(port)->isdu.request_len = 0U;
    iolink_master_port_state(port)->isdu.request_pos = 0U;
    iolink_master_port_state(port)->isdu.request_seq = 0U;
    iolink_master_port_state(port)->isdu.request_control_phase = true;
    iolink_master_port_state(port)->isdu.request_sent = false;
    iolink_master_port_state(port)->isdu.response_len = 0U;
    iolink_master_port_state(port)->isdu.response_seq = 0U;
    iolink_master_port_state(port)->isdu.response_expect_control = true;
    iolink_master_port_state(port)->isdu.response_last = false;
    iolink_master_port_state(port)->isdu.done = false;
    iolink_master_port_state(port)->isdu.error = IOLINK_ISDU_ERROR_NONE;
}

static void iolink_master_isdu_start(iolink_master_port_t* port,
                                     iolink_master_isdu_op_t op,
                                     uint16_t index,
                                     uint8_t subindex)
{
    iolink_master_port_state(port)->isdu.op = op;
    iolink_master_port_state(port)->isdu.index = index;
    iolink_master_port_state(port)->isdu.subindex = subindex;
    iolink_master_port_state(port)->isdu.request_pos = 0U;
    iolink_master_port_state(port)->isdu.request_seq = 0U;
    iolink_master_port_state(port)->isdu.request_control_phase = true;
    iolink_master_port_state(port)->isdu.request_sent = false;
    iolink_master_port_state(port)->isdu.response_len = 0U;
    iolink_master_port_state(port)->isdu.response_seq = 0U;
    iolink_master_port_state(port)->isdu.response_expect_control = true;
    iolink_master_port_state(port)->isdu.response_last = false;
    iolink_master_port_state(port)->isdu.done = false;
    iolink_master_port_state(port)->isdu.error = IOLINK_ISDU_ERROR_NONE;
}

static int iolink_master_service_result(iolink_master_port_t* port, int ret)
{
    if(ret != IOLINK_MASTER_STATUS_PENDING)
    {
        iolink_master_port_state(port)->diagnostics.last_service_result = ret;
    }

    return ret;
}

static int iolink_master_isdu_finish_read(iolink_master_port_t* port,
                                          uint8_t* data,
                                          uint8_t* len)
{
    uint16_t result_len = iolink_master_port_state(port)->isdu.response_len;

    if(iolink_master_port_state(port)->isdu.error != IOLINK_ISDU_ERROR_NONE)
    {
        iolink_master_port_state(port)->diagnostics.last_isdu_error =
            iolink_master_port_state(port)->isdu.error;
        iolink_master_isdu_clear(port);
        return iolink_master_service_result(port, IOLINK_MASTER_ISDU_ERR_DEVICE);
    }

    if((result_len >= 2U) && (iolink_master_port_state(port)->isdu.response[0] == IOLINK_MASTER_ISDU_RESPONSE_ERROR))
    {
        iolink_master_port_state(port)->isdu.error = iolink_master_port_state(port)->isdu.response[1];
        iolink_master_port_state(port)->diagnostics.last_isdu_error =
            iolink_master_port_state(port)->isdu.error;
        iolink_master_isdu_clear(port);
        return iolink_master_service_result(port, IOLINK_MASTER_ISDU_ERR_DEVICE);
    }

    if(*len < result_len)
    {
        *len = (result_len > UINT8_MAX) ? UINT8_MAX : (uint8_t)result_len;
        return iolink_master_service_result(port, IOLINK_MASTER_ISDU_ERR_BUFFER_TOO_SMALL);
    }

    if(result_len > 0U)
    {
        (void)memcpy(data, iolink_master_port_state(port)->isdu.response, result_len);
    }
    /* Guarded above by `*len < result_len`, so result_len fits in the uint8 out-length. */
    *len = (uint8_t)result_len;
    iolink_master_isdu_clear(port);
    return iolink_master_service_result(port, IOLINK_MASTER_STATUS_OK);
}

static int iolink_master_isdu_finish_write(iolink_master_port_t* port)
{
    if(iolink_master_port_state(port)->isdu.error != IOLINK_ISDU_ERROR_NONE)
    {
        iolink_master_port_state(port)->diagnostics.last_isdu_error =
            iolink_master_port_state(port)->isdu.error;
        iolink_master_isdu_clear(port);
        return iolink_master_service_result(port, IOLINK_MASTER_ISDU_ERR_DEVICE);
    }

    if((iolink_master_port_state(port)->isdu.response_len >= 2U) && (iolink_master_port_state(port)->isdu.response[0] == IOLINK_MASTER_ISDU_RESPONSE_ERROR))
    {
        iolink_master_port_state(port)->isdu.error = iolink_master_port_state(port)->isdu.response[1];
        iolink_master_port_state(port)->diagnostics.last_isdu_error =
            iolink_master_port_state(port)->isdu.error;
        iolink_master_isdu_clear(port);
        return iolink_master_service_result(port, IOLINK_MASTER_ISDU_ERR_DEVICE);
    }

    iolink_master_isdu_clear(port);
    return iolink_master_service_result(port, IOLINK_MASTER_STATUS_OK);
}

void iolink_master_isdu_fill_od(iolink_master_port_t* port, uint8_t* od, uint8_t od_len)
{
    uint8_t i;
    uint8_t ctrl;

    if((port == NULL) || (od == NULL))
    {
        return;
    }

    (void)memset(od, 0, od_len);

    if((iolink_master_port_state(port)->isdu.op == IOLINK_MASTER_ISDU_OP_NONE) || iolink_master_port_state(port)->isdu.request_sent)
    {
        return;
    }

    for(i = 0U; i < od_len; i++)
    {
        if(iolink_master_port_state(port)->isdu.request_sent)
        {
            return;
        }

        if(iolink_master_port_state(port)->isdu.request_control_phase)
        {
            ctrl = (uint8_t)(iolink_master_port_state(port)->isdu.request_seq & IOLINK_ISDU_CTRL_SEQ_MASK);
            if(iolink_master_port_state(port)->isdu.request_pos == 0U)
            {
                ctrl |= IOLINK_ISDU_CTRL_START;
            }
            if((uint8_t)(iolink_master_port_state(port)->isdu.request_pos + 1U) >= iolink_master_port_state(port)->isdu.request_len)
            {
                ctrl |= IOLINK_ISDU_CTRL_LAST;
            }

            od[i] = ctrl;
            iolink_master_port_state(port)->isdu.request_control_phase = false;
        }
        else
        {
            od[i] = iolink_master_port_state(port)->isdu.request[iolink_master_port_state(port)->isdu.request_pos++];
            if(iolink_master_port_state(port)->isdu.request_pos >= iolink_master_port_state(port)->isdu.request_len)
            {
                iolink_master_port_state(port)->isdu.request_sent = true;
            }
            else
            {
                iolink_master_port_state(port)->isdu.request_seq =
                    (uint8_t)((iolink_master_port_state(port)->isdu.request_seq + 1U) & IOLINK_ISDU_CTRL_SEQ_MASK);
                iolink_master_port_state(port)->isdu.request_control_phase = true;
            }
        }
    }
}

void iolink_master_isdu_on_od(iolink_master_port_t* port, const uint8_t* od, uint8_t od_len)
{
    uint8_t i;
    uint8_t byte;
    uint8_t seq;

    if((port == NULL) || (od == NULL) || (iolink_master_port_state(port)->isdu.op == IOLINK_MASTER_ISDU_OP_NONE) ||
       iolink_master_port_state(port)->isdu.done)
    {
        return;
    }

    for(i = 0U; i < od_len; i++)
    {
        byte = od[i];

        if(iolink_master_port_state(port)->isdu.response_expect_control &&
           (iolink_master_port_state(port)->isdu.response_len == 0U) &&
           ((byte & IOLINK_ISDU_CTRL_START) == 0U))
        {
            continue;
        }

        if(iolink_master_port_state(port)->isdu.response_expect_control)
        {
            if((byte & IOLINK_ISDU_CTRL_START) != 0U)
            {
                iolink_master_port_state(port)->isdu.response_len = 0U;
                iolink_master_port_state(port)->isdu.response_seq = 0U;
            }

            seq = (uint8_t)(byte & IOLINK_ISDU_CTRL_SEQ_MASK);
            if(seq != iolink_master_port_state(port)->isdu.response_seq)
            {
                iolink_master_port_state(port)->isdu.error = IOLINK_ISDU_ERROR_SEGMENTATION;
                iolink_master_port_state(port)->isdu.done = true;
                return;
            }

            iolink_master_port_state(port)->isdu.response_last = ((byte & IOLINK_ISDU_CTRL_LAST) != 0U);
            iolink_master_port_state(port)->isdu.response_expect_control = false;
        }
        else
        {
            if(iolink_master_port_state(port)->isdu.response_len >= IOLINK_ISDU_BUFFER_SIZE)
            {
                iolink_master_port_state(port)->isdu.error = IOLINK_ISDU_ERROR_SEGMENTATION;
                iolink_master_port_state(port)->isdu.done = true;
                return;
            }

            iolink_master_port_state(port)->isdu.response[iolink_master_port_state(port)->isdu.response_len++] = byte;

            if(iolink_master_port_state(port)->isdu.response_last)
            {
                iolink_master_port_state(port)->isdu.done = true;
                return;
            }

            iolink_master_port_state(port)->isdu.response_seq =
                (uint8_t)((iolink_master_port_state(port)->isdu.response_seq + 1U) & IOLINK_ISDU_CTRL_SEQ_MASK);
            iolink_master_port_state(port)->isdu.response_expect_control = true;
        }
    }
}

int iolink_master_read_isdu(iolink_master_port_t* port,
                            uint16_t index,
                            uint8_t subindex,
                            uint8_t* data,
                            uint8_t* len)
{
    if((port == NULL) || (data == NULL) || (len == NULL))
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    if((iolink_master_port_state(port)->state != IOLINK_MASTER_STATE_OPERATE) &&
       (iolink_master_port_state(port)->state != IOLINK_MASTER_STATE_PREOPERATE))
    {
        return IOLINK_MASTER_ISDU_ERR_INVALID_STATE;
    }

    if(iolink_master_isdu_busy(port))
    {
        if(iolink_master_isdu_matches(port, IOLINK_MASTER_ISDU_OP_READ, index, subindex))
        {
            return IOLINK_MASTER_STATUS_PENDING;
        }
        return IOLINK_MASTER_ISDU_ERR_BUSY;
    }

    if(iolink_master_port_state(port)->isdu.done)
    {
        if(!iolink_master_isdu_matches(port, IOLINK_MASTER_ISDU_OP_READ, index, subindex))
        {
            return IOLINK_MASTER_ISDU_ERR_BUSY;
        }
        return iolink_master_isdu_finish_read(port, data, len);
    }

    iolink_master_isdu_start(port, IOLINK_MASTER_ISDU_OP_READ, index, subindex);
    iolink_master_port_state(port)->isdu.request[0] = (uint8_t)(IOLINK_ISDU_SERVICE_READ << IOLINK_MASTER_ISDU_SERVICE_SHIFT);
    iolink_master_port_state(port)->isdu.request[1] = (uint8_t)(index >> 8);
    iolink_master_port_state(port)->isdu.request[2] = (uint8_t)(index & 0xFFU);
    iolink_master_port_state(port)->isdu.request[3] = subindex;
    iolink_master_port_state(port)->isdu.request_len = IOLINK_MASTER_ISDU_READ_HEADER_LEN;

    return IOLINK_MASTER_STATUS_PENDING;
}

int iolink_master_read_device_info(iolink_master_port_t* port)
{
    uint8_t page[IOLINK_MASTER_DPP1_LEN];
    uint8_t len = sizeof(page);
    int ret;

    if(port == NULL)
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    ret = iolink_master_read_isdu(port, IOLINK_IDX_DIRECT_PARAMETERS_1, 0U, page, &len);
    if(ret != 0)
    {
        return ret;
    }

    ret = iolink_master_apply_direct_parameter_page1(port, page, len);
    if(ret != 0)
    {
        return ret;
    }

    return iolink_master_validate_device_info(port);
}

int iolink_master_write_isdu(iolink_master_port_t* port,
                             uint16_t index,
                             uint8_t subindex,
                             const uint8_t* data,
                             uint8_t len)
{
    uint8_t pos = 0U;

    if((port == NULL) || ((data == NULL) && (len > 0U)))
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    if((iolink_master_port_state(port)->state != IOLINK_MASTER_STATE_OPERATE) &&
       (iolink_master_port_state(port)->state != IOLINK_MASTER_STATE_PREOPERATE))
    {
        return IOLINK_MASTER_ISDU_ERR_INVALID_STATE;
    }

    if(iolink_master_isdu_busy(port))
    {
        if(iolink_master_isdu_matches(port, IOLINK_MASTER_ISDU_OP_WRITE, index, subindex))
        {
            return IOLINK_MASTER_STATUS_PENDING;
        }
        return IOLINK_MASTER_ISDU_ERR_BUSY;
    }

    if(iolink_master_port_state(port)->isdu.done)
    {
        if(!iolink_master_isdu_matches(port, IOLINK_MASTER_ISDU_OP_WRITE, index, subindex))
        {
            return IOLINK_MASTER_ISDU_ERR_BUSY;
        }
        return iolink_master_isdu_finish_write(port);
    }

    if(len > (uint8_t)(IOLINK_ISDU_BUFFER_SIZE - IOLINK_MASTER_ISDU_WRITE_HEADER_MAX))
    {
        return IOLINK_MASTER_ISDU_ERR_BUFFER_TOO_SMALL;
    }

    iolink_master_isdu_start(port, IOLINK_MASTER_ISDU_OP_WRITE, index, subindex);

    if(len >= IOLINK_MASTER_ISDU_LENGTH_NIBBLE_MAX)
    {
        iolink_master_port_state(port)->isdu.request[pos++] = (uint8_t)((IOLINK_ISDU_SERVICE_WRITE << IOLINK_MASTER_ISDU_SERVICE_SHIFT) | IOLINK_MASTER_ISDU_LENGTH_EXTENDED);
        iolink_master_port_state(port)->isdu.request[pos++] = len;
    }
    else
    {
        iolink_master_port_state(port)->isdu.request[pos++] = (uint8_t)((IOLINK_ISDU_SERVICE_WRITE << IOLINK_MASTER_ISDU_SERVICE_SHIFT) | len);
    }

    iolink_master_port_state(port)->isdu.request[pos++] = (uint8_t)(index >> 8);
    iolink_master_port_state(port)->isdu.request[pos++] = (uint8_t)(index & 0xFFU);
    iolink_master_port_state(port)->isdu.request[pos++] = subindex;

    if(len > 0U)
    {
        (void)memcpy(&iolink_master_port_state(port)->isdu.request[pos], data, len);
        pos = (uint8_t)(pos + len);
    }

    iolink_master_port_state(port)->isdu.request_len = pos;

    return IOLINK_MASTER_STATUS_PENDING;
}

int iolink_master_read_data_storage(iolink_master_port_t* port, uint8_t* data, uint8_t* len)
{
    return iolink_master_read_isdu(port, IOLINK_IDX_DATA_STORAGE, 0U, data, len);
}

int iolink_master_write_data_storage(iolink_master_port_t* port,
                                     const uint8_t* data,
                                     uint8_t len)
{
    return iolink_master_write_isdu(port, IOLINK_IDX_DATA_STORAGE, 0U, data, len);
}

int iolink_master_restore_data_storage(iolink_master_port_t* port,
                                       const uint8_t* data,
                                       uint8_t len)
{
    return iolink_master_write_parameter_block(port, IOLINK_IDX_DATA_STORAGE, 0U, data, len);
}

int iolink_master_verify_isdu(iolink_master_port_t* port,
                              uint16_t index,
                              uint8_t subindex,
                              const uint8_t* expected,
                              uint8_t len)
{
    uint8_t data[IOLINK_ISDU_BUFFER_SIZE];
    uint8_t read_len = UINT8_MAX;
    int ret;

    if((expected == NULL) && (len > 0U))
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    ret = iolink_master_read_isdu(port, index, subindex, data, &read_len);
    if(ret != IOLINK_MASTER_STATUS_OK)
    {
        return ret;
    }

    if((read_len != len) || ((len > 0U) && (memcmp(data, expected, len) != 0)))
    {
        return iolink_master_service_result(port, IOLINK_MASTER_ISDU_ERR_VERIFY_FAILED);
    }

    return iolink_master_service_result(port, IOLINK_MASTER_STATUS_OK);
}

static bool iolink_master_ds_next_record(const uint8_t* data,
                                         uint8_t len,
                                         uint8_t* pos,
                                         const uint8_t** record,
                                         uint8_t* record_len)
{
    uint8_t value_len;

    if((data == NULL) || (pos == NULL) || (record == NULL) || (record_len == NULL) ||
       (*pos > len) || ((uint8_t)(len - *pos) < IOLINK_MASTER_DS_RECORD_HEADER_LEN))
    {
        return false;
    }

    value_len = data[(uint8_t)(*pos + (IOLINK_MASTER_DS_RECORD_HEADER_LEN - 1U))];
    if(value_len > (uint8_t)(len - *pos - IOLINK_MASTER_DS_RECORD_HEADER_LEN))
    {
        return false;
    }

    *record = &data[*pos];
    *record_len = (uint8_t)(IOLINK_MASTER_DS_RECORD_HEADER_LEN + value_len);
    *pos = (uint8_t)(*pos + *record_len);
    return true;
}

static bool iolink_master_ds_image_contains_records(const uint8_t* actual,
                                                    uint8_t actual_len,
                                                    const uint8_t* expected,
                                                    uint8_t expected_len)
{
    uint8_t expected_pos = 0U;
    const uint8_t* expected_record;
    uint8_t expected_record_len;

    if((expected == NULL) && (expected_len > 0U))
    {
        return false;
    }

    while(expected_pos < expected_len)
    {
        uint8_t actual_pos = 0U;
        bool found = false;

        if(!iolink_master_ds_next_record(expected,
                                         expected_len,
                                         &expected_pos,
                                         &expected_record,
                                         &expected_record_len))
        {
            return false;
        }

        while(actual_pos < actual_len)
        {
            const uint8_t* actual_record;
            uint8_t actual_record_len;

            if(!iolink_master_ds_next_record(actual,
                                             actual_len,
                                             &actual_pos,
                                             &actual_record,
                                             &actual_record_len))
            {
                return false;
            }

            if((actual_record_len == expected_record_len) &&
               (memcmp(actual_record, expected_record, expected_record_len) == 0))
            {
                found = true;
                break;
            }
        }

        if(!found)
        {
            return false;
        }
    }

    return true;
}

static bool iolink_master_ds_image_is_valid(const uint8_t* data, uint8_t len)
{
    uint8_t pos = 0U;

    if((data == NULL) && (len > 0U))
    {
        return false;
    }

    while(pos < len)
    {
        const uint8_t* record;
        uint8_t record_len;

        if(!iolink_master_ds_next_record(data, len, &pos, &record, &record_len))
        {
            return false;
        }
        (void)record;
        (void)record_len;
    }

    return true;
}

int iolink_master_verify_data_storage(iolink_master_port_t* port,
                                      const uint8_t* expected,
                                      uint8_t len)
{
    uint8_t data[IOLINK_ISDU_BUFFER_SIZE];
    uint8_t read_len = UINT8_MAX;
    int ret;

    if((expected == NULL) && (len > 0U))
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    ret = iolink_master_read_isdu(port, IOLINK_IDX_DATA_STORAGE, 0U, data, &read_len);
    if(ret != IOLINK_MASTER_STATUS_OK)
    {
        return ret;
    }

    if(iolink_master_ds_image_is_valid(expected, len) &&
       iolink_master_ds_image_is_valid(data, read_len))
    {
        if(!iolink_master_ds_image_contains_records(data, read_len, expected, len))
        {
            return iolink_master_service_result(port, IOLINK_MASTER_ISDU_ERR_VERIFY_FAILED);
        }
    }
    else if((read_len != len) || ((len > 0U) && (memcmp(data, expected, len) != 0)))
    {
        return iolink_master_service_result(port, IOLINK_MASTER_ISDU_ERR_VERIFY_FAILED);
    }
    else
    {
        /* Raw image matched: fall through to the success result. */
    }

    return iolink_master_service_result(port, IOLINK_MASTER_STATUS_OK);
}

int iolink_master_read_detailed_device_status(iolink_master_port_t* port,
                                              uint8_t* data,
                                              uint8_t* len)
{
    return iolink_master_read_isdu(port, IOLINK_IDX_DETAILED_DEVICE_STATUS, 0U, data, len);
}

int iolink_master_read_event_code(iolink_master_port_t* port, uint16_t* event_code)
{
    uint8_t data[2] = {0U};
    uint8_t len = sizeof(data);
    int ret;

    if(event_code == NULL)
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    ret = iolink_master_read_isdu(port, IOLINK_IDX_SYSTEM_COMMAND, 0U, data, &len);
    if(ret != IOLINK_MASTER_STATUS_OK)
    {
        return ret;
    }

    if(len < sizeof(data))
    {
        return IOLINK_MASTER_ISDU_ERR_DEVICE;
    }

    *event_code = (uint16_t)(((uint16_t)data[0] << 8U) | data[1]);
    iolink_master_port_state(port)->diagnostics.last_event_code = *event_code;
    return IOLINK_MASTER_STATUS_OK;
}

int iolink_master_ack_event(iolink_master_port_t* port, uint16_t* event_code)
{
    return iolink_master_read_event_code(port, event_code);
}

static iolink_master_event_type_t iolink_master_event_type_from_qualifier(uint8_t qualifier)
{
    switch((uint8_t)((qualifier >> IOLINK_MASTER_EVENT_QUALIFIER_MODE_SHIFT) & IOLINK_MASTER_EVENT_QUALIFIER_MODE_MASK))
    {
    case IOLINK_MASTER_EVENT_MODE_NOTIFICATION:
        return IOLINK_MASTER_EVENT_TYPE_NOTIFICATION;
    case IOLINK_MASTER_EVENT_MODE_WARNING:
        return IOLINK_MASTER_EVENT_TYPE_WARNING;
    case IOLINK_MASTER_EVENT_MODE_ERROR:
        return IOLINK_MASTER_EVENT_TYPE_ERROR;
    default:
        return IOLINK_MASTER_EVENT_TYPE_UNKNOWN;
    }
}

int iolink_master_read_event_details(iolink_master_port_t* port,
                                     iolink_master_event_t* events,
                                     uint8_t max_events,
                                     uint8_t* out_count)
{
    uint8_t data[IOLINK_MASTER_MAX_EVENTS * IOLINK_MASTER_EVENT_ENTRY_LEN] = {0U};
    uint8_t len = sizeof(data);
    uint8_t count;
    uint8_t i;
    int ret;

    if((events == NULL) || (out_count == NULL))
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    ret = iolink_master_read_detailed_device_status(port, data, &len);
    if(ret != IOLINK_MASTER_STATUS_OK)
    {
        return ret;
    }

    if((len % IOLINK_MASTER_EVENT_ENTRY_LEN) != 0U)
    {
        return IOLINK_MASTER_ISDU_ERR_DEVICE;
    }

    count = (uint8_t)(len / IOLINK_MASTER_EVENT_ENTRY_LEN);
    *out_count = count;
    iolink_master_port_state(port)->diagnostics.last_event_count = count;
    iolink_master_port_state(port)->diagnostics.last_event_code = 0U;
    if(max_events < count)
    {
        return IOLINK_MASTER_ERR_BUFFER_TOO_SMALL;
    }

    for(i = 0U; i < count; i++)
    {
        events[i].qualifier = data[i * IOLINK_MASTER_EVENT_ENTRY_LEN];
        events[i].type = iolink_master_event_type_from_qualifier(events[i].qualifier);
        events[i].code = (uint16_t)(((uint16_t)data[(i * IOLINK_MASTER_EVENT_ENTRY_LEN) + 1U] << 8U) |
                                    data[(i * IOLINK_MASTER_EVENT_ENTRY_LEN) + 2U]);
    }
    if(count > 0U)
    {
        iolink_master_port_state(port)->diagnostics.last_event_code = events[count - 1U].code;
    }

    if(iolink_master_port_state(port)->config.event_handler != NULL)
    {
        for(i = 0U; i < count; i++)
        {
            iolink_master_port_state(port)->config.event_handler(
                iolink_master_port_state(port)->config.event_user, &events[i]);
        }
    }

    return IOLINK_MASTER_STATUS_OK;
}

static int iolink_master_write_system_command(iolink_master_port_t* port, uint8_t command)
{
    return iolink_master_write_isdu(port, IOLINK_IDX_SYSTEM_COMMAND, 0U, &command, 1U);
}

int iolink_master_begin_parameter_download(iolink_master_port_t* port)
{
    return iolink_master_write_system_command(port, IOLINK_CMD_PARAM_DOWNLOAD_START);
}

int iolink_master_end_parameter_download(iolink_master_port_t* port)
{
    return iolink_master_write_system_command(port, IOLINK_CMD_PARAM_DOWNLOAD_END);
}

int iolink_master_begin_parameter_upload(iolink_master_port_t* port)
{
    return iolink_master_write_system_command(port, IOLINK_CMD_PARAM_UPLOAD_START);
}

int iolink_master_end_parameter_upload(iolink_master_port_t* port)
{
    return iolink_master_write_system_command(port, IOLINK_CMD_PARAM_UPLOAD_END);
}

int iolink_master_store_parameter_download(iolink_master_port_t* port)
{
    return iolink_master_write_system_command(port, IOLINK_CMD_PARAM_DOWNLOAD_STORE);
}

static bool iolink_master_block_matches(const iolink_master_port_t* port,
                                        uint16_t index,
                                        uint8_t subindex,
                                        const uint8_t* data,
                                        uint8_t len)
{
    const iolink_master_block_state_t* block = &iolink_master_port_const_state(port)->block;

    return (block->index == index) && (block->subindex == subindex) && (block->len == len) &&
           ((len == 0U) || (memcmp(block->data, data, len) == 0));
}

static void iolink_master_block_clear(iolink_master_port_t* port)
{
    (void)memset(&iolink_master_port_state(port)->block, 0, sizeof(iolink_master_port_state(port)->block));
}

int iolink_master_write_parameter_block(iolink_master_port_t* port,
                                        uint16_t index,
                                        uint8_t subindex,
                                        const uint8_t* data,
                                        uint8_t len)
{
    iolink_master_block_state_t* block;
    int ret;

    if((port == NULL) || ((data == NULL) && (len > 0U)))
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    if(len > (uint8_t)(IOLINK_ISDU_BUFFER_SIZE - IOLINK_MASTER_ISDU_WRITE_HEADER_MAX))
    {
        return IOLINK_MASTER_ISDU_ERR_BUFFER_TOO_SMALL;
    }

    block = &iolink_master_port_state(port)->block;
    if(block->step == IOLINK_MASTER_BLOCK_STEP_NONE)
    {
        block->step = IOLINK_MASTER_BLOCK_STEP_BEGIN_DOWNLOAD;
        block->index = index;
        block->subindex = subindex;
        block->len = len;
        if(len > 0U)
        {
            (void)memcpy(block->data, data, len);
        }
    }
    else if(!iolink_master_block_matches(port, index, subindex, data, len))
    {
        return iolink_master_service_result(port, IOLINK_MASTER_ISDU_ERR_BUSY);
    }
    else
    {
        /* Resuming the same block transfer: keep the latched state. */
    }

    if(block->step == IOLINK_MASTER_BLOCK_STEP_BEGIN_DOWNLOAD)
    {
        ret = iolink_master_begin_parameter_download(port);
        if(ret != IOLINK_MASTER_STATUS_OK)
        {
            return ret;
        }
        block->step = IOLINK_MASTER_BLOCK_STEP_WRITE;
    }

    if(block->step == IOLINK_MASTER_BLOCK_STEP_WRITE)
    {
        ret = iolink_master_write_isdu(port, block->index, block->subindex, block->data, block->len);
        if(ret != IOLINK_MASTER_STATUS_OK)
        {
            return ret;
        }
        block->step = IOLINK_MASTER_BLOCK_STEP_END_DOWNLOAD;
    }

    if(block->step == IOLINK_MASTER_BLOCK_STEP_END_DOWNLOAD)
    {
        ret = iolink_master_end_parameter_download(port);
        if(ret != IOLINK_MASTER_STATUS_OK)
        {
            return ret;
        }
        block->step = IOLINK_MASTER_BLOCK_STEP_VERIFY;
    }

    if((block->index == IOLINK_IDX_DATA_STORAGE) && (block->subindex == 0U))
    {
        ret = iolink_master_verify_data_storage(port, block->data, block->len);
    }
    else
    {
        ret = iolink_master_verify_isdu(port, block->index, block->subindex, block->data, block->len);
    }
    if(ret == IOLINK_MASTER_STATUS_OK)
    {
        iolink_master_block_clear(port);
    }
    else if(ret < 0)
    {
        iolink_master_block_clear(port);
    }
    else
    {
        /* Still pending: keep the block state for the next call. */
    }

    return iolink_master_service_result(port, ret);
}
