#include "fake_iolink_device.h"

#include "iolinki/crc.h"
#include "iolinki/frame.h"
#include "iolinki/protocol.h"

#include <stdbool.h>
#include <stddef.h>
#include <string.h>

#define FAKE_IOLINK_DEVICE_OBJECT_MAX_LEN 16U
#define FAKE_IOLINK_DEVICE_OBJECT_MAX_COUNT 4U
#define FAKE_IOLINK_DEVICE_ISDU_REQUEST_MAX_LEN 8U

typedef struct
{
    uint16_t index;
    uint8_t subindex;
    uint8_t data[FAKE_IOLINK_DEVICE_OBJECT_MAX_LEN];
    uint8_t len;
    bool valid;
} fake_iolink_device_object_t;

typedef struct
{
    uint8_t pd_in_value;
    uint8_t pd_in_len;
    uint8_t od_len;
    bool event_pending;
    uint8_t rx_queue[16];
    uint8_t rx_len;
    uint8_t rx_pos;
    uint32_t wakeup_count;
    uint32_t transition_count;
    uint32_t operate_cycle_count;
    bool corrupt_next_response_checksum;
    bool drop_next_response;
    bool truncate_next_response;
    fake_iolink_device_object_t objects[FAKE_IOLINK_DEVICE_OBJECT_MAX_COUNT];
    uint8_t object_count;
    uint8_t isdu_request[FAKE_IOLINK_DEVICE_ISDU_REQUEST_MAX_LEN];
    uint8_t isdu_request_len;
    bool isdu_request_expect_data;
    bool isdu_request_last_control;
    uint8_t isdu_response[FAKE_IOLINK_DEVICE_OBJECT_MAX_LEN];
    uint8_t isdu_response_len;
    uint8_t isdu_response_pos;
    bool isdu_response_active;
} fake_iolink_device_t;

static fake_iolink_device_t g_device;

static fake_iolink_device_object_t* fake_iolink_device_find_object(uint16_t index, uint8_t subindex)
{
    uint8_t i;

    for(i = 0U; i < g_device.object_count; i++)
    {
        if(g_device.objects[i].valid && (g_device.objects[i].index == index) &&
           (g_device.objects[i].subindex == subindex))
        {
            return &g_device.objects[i];
        }
    }

    return NULL;
}

static fake_iolink_device_object_t* fake_iolink_device_find_or_create_object(uint16_t index, uint8_t subindex)
{
    fake_iolink_device_object_t* object;

    object = fake_iolink_device_find_object(index, subindex);
    if((object == NULL) && (g_device.object_count < FAKE_IOLINK_DEVICE_OBJECT_MAX_COUNT))
    {
        object = &g_device.objects[g_device.object_count++];
        object->index = index;
        object->subindex = subindex;
    }

    return object;
}

static void fake_iolink_device_prepare_isdu_error(uint8_t error)
{
    g_device.isdu_response[0] = 0x80U;
    g_device.isdu_response[1] = error;
    g_device.isdu_response_len = 2U;
    g_device.isdu_response_pos = 0U;
    g_device.isdu_response_active = true;
}

static void fake_iolink_device_prepare_isdu_ack(void)
{
    g_device.isdu_response[0] = 0x00U;
    g_device.isdu_response_len = 1U;
    g_device.isdu_response_pos = 0U;
    g_device.isdu_response_active = true;
}

static void fake_iolink_device_prepare_isdu_response(void)
{
    uint8_t service;
    uint16_t index;
    uint8_t subindex;
    uint8_t len;
    fake_iolink_device_object_t* object;

    if(g_device.isdu_request_len < 4U)
    {
        fake_iolink_device_prepare_isdu_error(IOLINK_ISDU_ERROR_SERVICE_NOT_AVAIL);
        return;
    }

    service = (uint8_t)(g_device.isdu_request[0] >> 4);
    index = (uint16_t)(((uint16_t)g_device.isdu_request[1] << 8) | g_device.isdu_request[2]);
    subindex = g_device.isdu_request[3];

    if(service == IOLINK_ISDU_SERVICE_READ)
    {
        object = fake_iolink_device_find_object(index, subindex);
        if(object == NULL)
        {
            fake_iolink_device_prepare_isdu_error(IOLINK_ISDU_ERROR_SERVICE_NOT_AVAIL);
            return;
        }

        memcpy(g_device.isdu_response, object->data, object->len);
        g_device.isdu_response_len = object->len;
        g_device.isdu_response_pos = 0U;
        g_device.isdu_response_active = true;
        return;
    }

    if(service == IOLINK_ISDU_SERVICE_WRITE)
    {
        len = (uint8_t)(g_device.isdu_request[0] & 0x0FU);
        if((len == 0x0FU) || (g_device.isdu_request_len < (uint8_t)(4U + len)) ||
           (len > FAKE_IOLINK_DEVICE_OBJECT_MAX_LEN))
        {
            fake_iolink_device_prepare_isdu_error(IOLINK_ISDU_ERROR_SERVICE_NOT_AVAIL);
            return;
        }

        object = fake_iolink_device_find_or_create_object(index, subindex);
        if(object == NULL)
        {
            fake_iolink_device_prepare_isdu_error(IOLINK_ISDU_ERROR_SERVICE_NOT_AVAIL);
            return;
        }

        memcpy(object->data, &g_device.isdu_request[4], len);
        object->len = len;
        object->valid = true;
        fake_iolink_device_prepare_isdu_ack();
        return;
    }

    fake_iolink_device_prepare_isdu_error(IOLINK_ISDU_ERROR_SERVICE_NOT_AVAIL);
}

static void fake_iolink_device_on_master_od(uint8_t od)
{
    if(!g_device.isdu_request_expect_data)
    {
        if((g_device.isdu_request_len == 0U) && ((od & IOLINK_ISDU_CTRL_START) == 0U))
        {
            return;
        }

        if((od & IOLINK_ISDU_CTRL_START) != 0U)
        {
            g_device.isdu_request_len = 0U;
        }

        g_device.isdu_request_last_control = ((od & IOLINK_ISDU_CTRL_LAST) != 0U);
        g_device.isdu_request_expect_data = true;
        return;
    }

    if(g_device.isdu_request_len < FAKE_IOLINK_DEVICE_ISDU_REQUEST_MAX_LEN)
    {
        g_device.isdu_request[g_device.isdu_request_len++] = od;
    }

    if(g_device.isdu_request_last_control)
    {
        fake_iolink_device_prepare_isdu_response();
        g_device.isdu_request_len = 0U;
    }

    g_device.isdu_request_expect_data = false;
}

static uint8_t fake_iolink_device_next_response_od(void)
{
    uint8_t data_index;
    uint8_t od;

    if(!g_device.isdu_response_active || (g_device.isdu_response_len == 0U))
    {
        return 0U;
    }

    data_index = (uint8_t)(g_device.isdu_response_pos / 2U);
    if((g_device.isdu_response_pos & 1U) == 0U)
    {
        od = (uint8_t)(data_index & IOLINK_ISDU_CTRL_SEQ_MASK);
        if(data_index == 0U)
        {
            od |= IOLINK_ISDU_CTRL_START;
        }
        if((uint8_t)(data_index + 1U) >= g_device.isdu_response_len)
        {
            od |= IOLINK_ISDU_CTRL_LAST;
        }
    }
    else
    {
        od = g_device.isdu_response[data_index];
    }

    g_device.isdu_response_pos++;
    if(g_device.isdu_response_pos >= (uint8_t)(g_device.isdu_response_len * 2U))
    {
        g_device.isdu_response_active = false;
    }

    return od;
}

static void fake_iolink_device_queue_type0(uint8_t value)
{
    if(g_device.drop_next_response)
    {
        g_device.rx_len = 0U;
        g_device.rx_pos = 0U;
        g_device.drop_next_response = false;
        return;
    }

    g_device.rx_queue[0] = value;
    g_device.rx_queue[1] = iolink_checksum_ck(value, 0U);
    if(g_device.corrupt_next_response_checksum)
    {
        g_device.rx_queue[1] ^= 0x01U;
        g_device.corrupt_next_response_checksum = false;
    }
    g_device.rx_len = IOLINK_M_SEQ_TYPE0_LEN;
    if(g_device.truncate_next_response && (g_device.rx_len > 0U))
    {
        g_device.rx_len--;
        g_device.truncate_next_response = false;
    }
    g_device.rx_pos = 0U;
}

static void fake_iolink_device_queue_operate_response(void)
{
    uint8_t pos = 0U;
    uint8_t i;

    if(g_device.drop_next_response)
    {
        g_device.rx_len = 0U;
        g_device.rx_pos = 0U;
        g_device.drop_next_response = false;
        return;
    }

    g_device.rx_queue[pos++] = IOLINK_OD_STATUS_PD_VALID | IOLINK_DEVICE_STATUS_OK |
                               (g_device.event_pending ? IOLINK_OD_STATUS_EVENT : 0U);

    for(i = 0U; i < g_device.pd_in_len; i++)
    {
        g_device.rx_queue[pos++] = g_device.pd_in_value;
    }

    for(i = 0U; i < g_device.od_len; i++)
    {
        g_device.rx_queue[pos++] = fake_iolink_device_next_response_od();
    }

    g_device.rx_queue[pos] = iolink_crc6(g_device.rx_queue, pos);
    if(g_device.corrupt_next_response_checksum)
    {
        g_device.rx_queue[pos] ^= 0x01U;
        g_device.corrupt_next_response_checksum = false;
    }
    g_device.rx_len = (uint8_t)(pos + 1U);
    if(g_device.truncate_next_response && (g_device.rx_len > 0U))
    {
        g_device.rx_len--;
        g_device.truncate_next_response = false;
    }
    g_device.rx_pos = 0U;
}

static int fake_iolink_device_send(const uint8_t* data, size_t len)
{
    if((data == NULL) || (len == 0U))
    {
        return -1;
    }

    if((len == 1U) && (data[0] == 0x55U))
    {
        g_device.wakeup_count++;
        return (int)len;
    }

    if(len == IOLINK_M_SEQ_TYPE0_LEN)
    {
        if(data[0] == IOLINK_MC_TRANSITION_COMMAND)
        {
            g_device.transition_count++;
            return (int)len;
        }

        fake_iolink_device_on_master_od(data[0]);
        fake_iolink_device_queue_type0(fake_iolink_device_next_response_od());
        return (int)len;
    }

    g_device.operate_cycle_count++;
    if(len > IOLINK_M_SEQ_HEADER_LEN)
    {
        fake_iolink_device_on_master_od(data[IOLINK_M_SEQ_HEADER_LEN]);
    }
    fake_iolink_device_queue_operate_response();
    return (int)len;
}

static int fake_iolink_device_recv_byte(uint8_t* byte)
{
    if(byte == NULL)
    {
        return -1;
    }

    if(g_device.rx_pos >= g_device.rx_len)
    {
        return 0;
    }

    *byte = g_device.rx_queue[g_device.rx_pos++];
    return 1;
}

static const iolink_phy_api_t g_phy = {
    .send = fake_iolink_device_send,
    .recv_byte = fake_iolink_device_recv_byte,
};

void fake_iolink_device_reset(uint8_t pd_in_value, uint8_t pd_in_len, uint8_t od_len)
{
    memset(&g_device, 0, sizeof(g_device));
    g_device.pd_in_value = pd_in_value;
    g_device.pd_in_len = pd_in_len;
    g_device.od_len = od_len;
}

void fake_iolink_device_set_isdu_object(uint16_t index, uint8_t subindex, const uint8_t* data, uint8_t len)
{
    fake_iolink_device_object_t* object;

    if((data == NULL) || (len == 0U) || (len > FAKE_IOLINK_DEVICE_OBJECT_MAX_LEN))
    {
        return;
    }

    object = fake_iolink_device_find_or_create_object(index, subindex);
    if(object == NULL)
    {
        return;
    }

    memcpy(object->data, data, len);
    object->len = len;
    object->valid = true;
}

void fake_iolink_device_set_direct_parameter_page1(uint8_t min_cycle_time,
                                                   uint8_t mseq_capability,
                                                   uint8_t pd_in_descriptor,
                                                   uint8_t pd_out_descriptor,
                                                   uint16_t vendor_id,
                                                   uint32_t device_id)
{
    uint8_t page[16] = {0U};

    page[0x02] = min_cycle_time;
    page[0x03] = mseq_capability;
    page[0x04] = 0x11U;
    page[0x05] = pd_in_descriptor;
    page[0x06] = pd_out_descriptor;
    page[0x07] = (uint8_t)(vendor_id >> 8);
    page[0x08] = (uint8_t)(vendor_id & 0xFFU);
    page[0x09] = (uint8_t)((device_id >> 16) & 0xFFU);
    page[0x0A] = (uint8_t)((device_id >> 8) & 0xFFU);
    page[0x0B] = (uint8_t)(device_id & 0xFFU);

    fake_iolink_device_set_isdu_object(IOLINK_IDX_DIRECT_PARAMETERS_1, 0U, page, sizeof(page));
}

void fake_iolink_device_set_data_storage(const uint8_t* data, uint8_t len)
{
    fake_iolink_device_set_isdu_object(IOLINK_IDX_DATA_STORAGE, 0U, data, len);
}

void fake_iolink_device_set_event_pending(bool pending)
{
    g_device.event_pending = pending;
}

void fake_iolink_device_set_event_code(uint16_t event_code)
{
    uint8_t data[2];

    data[0] = (uint8_t)(event_code >> 8);
    data[1] = (uint8_t)(event_code & 0xFFU);
    fake_iolink_device_set_isdu_object(IOLINK_IDX_SYSTEM_COMMAND, 0U, data, sizeof(data));
}

void fake_iolink_device_corrupt_next_response_checksum(void)
{
    g_device.corrupt_next_response_checksum = true;
}

void fake_iolink_device_drop_next_response(void)
{
    g_device.drop_next_response = true;
}

void fake_iolink_device_truncate_next_response(void)
{
    g_device.truncate_next_response = true;
}

const iolink_phy_api_t* fake_iolink_device_phy(void)
{
    return &g_phy;
}

uint32_t fake_iolink_device_wakeup_count(void)
{
    return g_device.wakeup_count;
}

uint32_t fake_iolink_device_transition_count(void)
{
    return g_device.transition_count;
}

uint32_t fake_iolink_device_operate_cycle_count(void)
{
    return g_device.operate_cycle_count;
}
