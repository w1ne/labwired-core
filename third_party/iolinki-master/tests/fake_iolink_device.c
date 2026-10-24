#include "fake_iolink_device.h"

#include "iolinki/crc.h"
#include "iolinki/frame.h"
#include "iolinki/protocol.h"

#include <stdbool.h>
#include <stddef.h>
#include <string.h>

#define FAKE_IOLINK_DEVICE_OBJECT_MAX_LEN 16U
#define FAKE_IOLINK_DEVICE_OBJECT_MAX_COUNT 4U
#define FAKE_IOLINK_DEVICE_ISDU_REQUEST_MAX_LEN 80U

/* Coarse link state, used to disambiguate the startup probe (a page-channel
   Type-0 read seen only once, right after wake-up) from later PREOPERATE ISDU
   traffic that can carry the same MC bit pattern. */
#define FAKE_LINK_STARTUP 0U
#define FAKE_LINK_PREOPERATE 1U
#define FAKE_LINK_OPERATE 2U

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
    uint8_t link_state;
    uint32_t wakeup_count;
    uint32_t transition_count;
    uint32_t operate_cycle_count;
    bool corrupt_next_response_checksum;
    bool drop_next_response;
    bool truncate_next_response;
    bool keep_unread_responses;
    fake_iolink_device_object_t objects[FAKE_IOLINK_DEVICE_OBJECT_MAX_COUNT];
    uint8_t object_count;
    uint8_t isdu_request[FAKE_IOLINK_DEVICE_ISDU_REQUEST_MAX_LEN];
    uint8_t isdu_request_len;
    bool isdu_request_expect_data;
    bool isdu_request_last_control;
    uint8_t isdu_response[128];
    uint8_t isdu_response_len;
    uint8_t isdu_response_pos;
    bool isdu_response_active;
    uint8_t event_memory[19];
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

/** @brief Append the XOR of every octet as the final CHKPDU (A.5.6). */
static void fake_iolink_device_isdu_append_chkpdu(void)
{
    uint8_t chk = 0U;
    uint8_t i;

    for(i = 0U; i < g_device.isdu_response_len; i++)
    {
        chk ^= g_device.isdu_response[i];
    }
    g_device.isdu_response[g_device.isdu_response_len++] = chk;
}

/** @brief Build a negative response carrying ErrorType = ErrorCode, AdditionalCode. */
static void fake_iolink_device_prepare_isdu_error(uint8_t error_code, uint8_t additional_code)
{
    g_device.isdu_response[0] = 0xC4U; /* Read/Write Response (-), Length = 4 */
    g_device.isdu_response[1] = error_code;
    g_device.isdu_response[2] = additional_code;
    g_device.isdu_response_len = 3U;
    fake_iolink_device_isdu_append_chkpdu();
    g_device.isdu_response_pos = 0U;
    g_device.isdu_response_active = true;
}

/** @brief Build the positive ack of a Write Request (Table A.13). */
static void fake_iolink_device_prepare_isdu_ack(void)
{
    g_device.isdu_response[0] = 0x52U; /* Write Response (+), Length = 2 */
    g_device.isdu_response_len = 1U;
    fake_iolink_device_isdu_append_chkpdu();
    g_device.isdu_response_pos = 0U;
    g_device.isdu_response_active = true;
}

/** @brief Build a positive Read Response (+) carrying @p data (Table A.13). */
static void fake_iolink_device_prepare_isdu_read_data(const uint8_t* data, uint8_t len)
{
    uint16_t total = (uint16_t)(1U + len + 1U);

    if(total <= 15U)
    {
        g_device.isdu_response[0] = (uint8_t)(0xD0U | (uint8_t)total);
        memcpy(&g_device.isdu_response[1], data, len);
        g_device.isdu_response_len = (uint8_t)(1U + len);
    }
    else
    {
        /* Extended form adds the ExtLength octet to the total (A.5.3). */
        total = (uint16_t)(2U + len + 1U);
        g_device.isdu_response[0] = 0xD1U; /* Length = 1 selects ExtLength (A.5.3) */
        g_device.isdu_response[1] = (uint8_t)total;
        memcpy(&g_device.isdu_response[2], data, len);
        g_device.isdu_response_len = (uint8_t)(2U + len);
    }

    fake_iolink_device_isdu_append_chkpdu();
    g_device.isdu_response_pos = 0U;
    g_device.isdu_response_active = true;
}

/** @brief Decode the accumulated ISDU request stream and stage the response. */
static void fake_iolink_device_prepare_isdu_response(void)
{
    uint8_t service;
    uint16_t index;
    uint8_t subindex;
    uint8_t index_len;
    uint8_t payload_start;
    uint8_t payload_len;
    fake_iolink_device_object_t* object;

    if(g_device.isdu_request_len < 2U)
    {
        fake_iolink_device_prepare_isdu_error(0x80U, IOLINK_ISDU_ERROR_SERVICE_NOT_AVAIL);
        return;
    }

    service = (uint8_t)(g_device.isdu_request[0] >> 4);
    index_len = (uint8_t)(service & 0x07U);

    if((index_len == 0U) || (g_device.isdu_request_len < (uint8_t)(2U + index_len)))
    {
        fake_iolink_device_prepare_isdu_error(0x80U, IOLINK_ISDU_ERROR_SERVICE_NOT_AVAIL);
        return;
    }

    if(index_len == 1U)
    {
        index = g_device.isdu_request[1];
        subindex = 0U;
    }
    else if(index_len == 2U)
    {
        index = g_device.isdu_request[1];
        subindex = g_device.isdu_request[2];
    }
    else
    {
        index = (uint16_t)(((uint16_t)g_device.isdu_request[1] << 8) | g_device.isdu_request[2]);
        subindex = g_device.isdu_request[3];
    }

    /* Accept the spec Table A.12 read I-Service codes (0x9/0xA/0xB). */
    if((service == 0x09U) || (service == 0x0AU) || (service == 0x0BU))
    {
        object = fake_iolink_device_find_object(index, subindex);
        if(object == NULL)
        {
            fake_iolink_device_prepare_isdu_error(0x80U, IOLINK_ISDU_ERROR_SERVICE_NOT_AVAIL);
            return;
        }

        fake_iolink_device_prepare_isdu_read_data(object->data, object->len);
        return;
    }

    /* Accept the spec Table A.12 write I-Service codes (0x1/0x2/0x3). */
    if((service == 0x01U) || (service == 0x02U) || (service == 0x03U))
    {
        uint8_t total = g_device.isdu_request_len;

        /* A.5.6: the last octet is CHKPDU. */
        payload_start = (uint8_t)(1U + index_len);
        payload_len = (uint8_t)(total - payload_start - 1U);
        if(payload_len > FAKE_IOLINK_DEVICE_OBJECT_MAX_LEN)
        {
            fake_iolink_device_prepare_isdu_error(0x80U, IOLINK_ISDU_ERROR_SERVICE_NOT_AVAIL);
            return;
        }

        object = fake_iolink_device_find_or_create_object(index, subindex);
        if(object == NULL)
        {
            fake_iolink_device_prepare_isdu_error(0x80U, IOLINK_ISDU_ERROR_SERVICE_NOT_AVAIL);
            return;
        }

        if(payload_len > 0U)
        {
            memcpy(object->data, &g_device.isdu_request[payload_start], payload_len);
        }
        object->len = payload_len;
        object->valid = true;
        fake_iolink_device_prepare_isdu_ack();
        return;
    }

    fake_iolink_device_prepare_isdu_error(0x80U, IOLINK_ISDU_ERROR_SERVICE_NOT_AVAIL);
}

/** @brief Accrue one ISDU request octet and stage the response once complete. */
static void fake_iolink_device_on_master_od(uint8_t od)
{
    uint16_t total;
    uint16_t declared;

    if(g_device.isdu_request_len < FAKE_IOLINK_DEVICE_ISDU_REQUEST_MAX_LEN)
    {
        g_device.isdu_request[g_device.isdu_request_len++] = od;
    }

    if(g_device.isdu_request_len < 2U)
    {
        return;
    }

    declared = (uint16_t)(g_device.isdu_request[0] & 0x0FU);
    if(declared == 1U)
    {
        declared = g_device.isdu_request[1];
    }
    if(declared < 2U)
    {
        return;
    }

    total = g_device.isdu_request_len;
    if(total >= declared)
    {
        fake_iolink_device_prepare_isdu_response();
        g_device.isdu_request_len = 0U;
    }
}

/** @brief Return the next octet of the staged ISDU response stream (0 when idle). */
static uint8_t fake_iolink_device_next_response_od(void)
{
    uint8_t od;

    if(!g_device.isdu_response_active || (g_device.isdu_response_pos >= g_device.isdu_response_len))
    {
        return 0U;
    }

    od = g_device.isdu_response[g_device.isdu_response_pos++];
    return od;
}

/** @brief Queue start position for the next reply.
 *
 * By default a new reply replaces any unread bytes. With
 * fake_iolink_device_keep_unread_responses() armed, unread bytes are kept and
 * the new reply is appended after them, in wire order.
 */
static uint8_t fake_iolink_device_queue_begin(void)
{
    if(g_device.keep_unread_responses && (g_device.rx_pos < g_device.rx_len))
    {
        return g_device.rx_len;
    }

    g_device.rx_pos = 0U;
    g_device.rx_len = 0U;
    return 0U;
}

static void fake_iolink_device_queue_type0(uint8_t value)
{
    uint8_t pos;

    if(g_device.drop_next_response)
    {
        g_device.rx_len = 0U;
        g_device.rx_pos = 0U;
        g_device.drop_next_response = false;
        return;
    }

    pos = fake_iolink_device_queue_begin();

    /* A.1.5 TYPE_0 reply: one OD octet plus CKS (A.1.6 checksum over [data, CKS=0]). */
    g_device.rx_queue[pos] = value;
    g_device.rx_queue[pos + 1U] = 0x00U;
    g_device.rx_queue[pos + 1U] = iolink_checksum6(&g_device.rx_queue[pos], 2U);
    if(g_device.corrupt_next_response_checksum)
    {
        g_device.rx_queue[pos + 1U] ^= 0x01U;
        g_device.corrupt_next_response_checksum = false;
    }
    g_device.rx_len = (uint8_t)(pos + IOLINK_M_SEQ_TYPE0_LEN);
    if(g_device.truncate_next_response && (g_device.rx_len > 0U))
    {
        g_device.rx_len--;
        g_device.truncate_next_response = false;
    }
}

static void fake_iolink_device_queue_operate_response(bool read)
{
    uint8_t start;
    uint8_t pos;
    uint8_t i;

    if(g_device.drop_next_response)
    {
        g_device.rx_len = 0U;
        g_device.rx_pos = 0U;
        g_device.drop_next_response = false;
        return;
    }

    start = fake_iolink_device_queue_begin();
    pos = start;

    /* Figures A.9/A.10/A.13/A.14: a READ reply is [OD octets][PD-in octets]
       CKS with the service response in the OD; a WRITE reply is
       [PD-in octets] CKS with no OD. CKS carries the Event flag in bit 7 and
       PD-invalid in bit 6 (A.1.5). */
    if(read)
    {
        for(i = 0U; i < g_device.od_len; i++)
        {
            g_device.rx_queue[pos++] = fake_iolink_device_next_response_od();
        }
    }

    for(i = 0U; i < g_device.pd_in_len; i++)
    {
        g_device.rx_queue[pos++] = g_device.pd_in_value;
    }

    /* A.1.5: CKS carries the Event flag in bit 7 (0x80). */
    g_device.rx_queue[pos] = (uint8_t)(g_device.event_pending ? 0x80U : 0U);
    g_device.rx_queue[pos] =
        (uint8_t)(iolink_checksum6(&g_device.rx_queue[start], (size_t)(pos + 1U - start)) |
                  g_device.rx_queue[pos]);
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
}

static uint8_t fake_iolink_device_direct_param_octet(uint8_t addr)
{
    fake_iolink_device_object_t* page =
        fake_iolink_device_find_object(IOLINK_IDX_DIRECT_PARAMETERS_1, 0U);
    if((page != NULL) && (addr < page->len))
    {
        return page->data[addr];
    }
    return 0U;
}

/** @brief Return one octet of the Table 58 event memory served on DIAGNOSIS. */
static uint8_t fake_iolink_device_event_memory_octet(uint8_t addr)
{
    return (addr < sizeof(g_device.event_memory)) ? g_device.event_memory[addr] : 0U;
}

/** @brief Queue an OPERATE reply for a DIAGNOSIS event-memory read.
 *
 * A diagnosis read is a READ M-sequence: the reply is [OD octets][PD-in
 * octets] CKS (Figures A.9/A.10/A.13/A.14), with the CKS Event flag in bit 7.
 * @p od_len is the port's OD width (1 for TYPE_0/TYPE_2, wider for TYPE_1 with
 * interleaved PD).
 */
static void fake_iolink_device_queue_diagnosis_read(uint8_t addr, uint8_t od_len)
{
    uint8_t start;
    uint8_t pos;
    uint8_t i;

    if(od_len == 0U)
    {
        od_len = 1U;
    }

    start = fake_iolink_device_queue_begin();
    pos = start;

    for(i = 0U; i < od_len; i++)
    {
        g_device.rx_queue[pos++] =
            fake_iolink_device_event_memory_octet((uint8_t)(addr + i));
    }
    for(i = 0U; i < g_device.pd_in_len; i++)
    {
        g_device.rx_queue[pos++] = g_device.pd_in_value;
    }

    g_device.rx_queue[pos] = (uint8_t)(g_device.event_pending ? 0x80U : 0U);
    g_device.rx_queue[pos] =
        (uint8_t)(iolink_checksum6(&g_device.rx_queue[start], (size_t)(pos + 1U - start)) |
                  g_device.rx_queue[pos]);
    g_device.rx_len = (uint8_t)(pos + 1U);
}

static int fake_iolink_device_send(void* user, const uint8_t* data, size_t len)
{
    (void)user;
    if((data == NULL) || (len == 0U))
    {
        return -1;
    }

    if((len == 1U) && (data[0] == 0x55U))
    {
        g_device.wakeup_count++;
        return (int)len;
    }

    /* DIAGNOSIS channel (7.3.8, Table 58): a read returns the event-memory octet
       at the MC address; a write to address 0 confirms the readout. */
    if((data[0] & IOLINK_MC_COMM_CHANNEL_MASK) == IOLINK_MC_CHANNEL_DIAGNOSIS)
    {
        if((data[0] & IOLINK_MC_RW_MASK) != 0U)
        {
            fake_iolink_device_queue_diagnosis_read((uint8_t)(data[0] & IOLINK_MC_ADDR_MASK),
                                                    g_device.od_len);
        }
        else
        {
            /* StatusCode confirmation: the device release is not gated on a
               reply octet (Table 59 T8); clear the Event flag. */
            g_device.event_pending = false;
        }
        return (int)len;
    }

    /* ISDU channel (A.1.2, Table A.1): FlowCTRL lives in the MC address bits.
       A write M-sequence carries request octets; every message is answered. */
    if((data[0] & IOLINK_MC_COMM_CHANNEL_MASK) == IOLINK_MC_CHANNEL_ISDU)
    {
        size_t i;

        if((data[0] & IOLINK_MC_RW_MASK) == 0U)
        {
            for(i = IOLINK_M_SEQ_HEADER_LEN; i < len; i++)
            {
                fake_iolink_device_on_master_od(data[i]);
            }
        }

        /* A write M-sequence is answered with "no service" (Table A.14); only a
           read M-sequence delivers response octets. */
        if(g_device.link_state == FAKE_LINK_OPERATE)
        {
            fake_iolink_device_queue_operate_response((data[0] & IOLINK_MC_RW_MASK) != 0U);
        }
        else
        {
            fake_iolink_device_queue_type0((data[0] & IOLINK_MC_RW_MASK) != 0U
                                               ? fake_iolink_device_next_response_od()
                                               : 0U);
        }
        return (int)len;
    }

    /* Spec DeviceOperate: Type-0 WRITE of MasterCommand 0x99 to Direct Parameter
       address 0x00 on the page channel (MC 0x20). Establishes communication. */
    if((len == IOLINK_M_SEQ_MIN_LEN) && (data[0] == 0x20U) &&
       (data[IOLINK_M_SEQ_HEADER_LEN] == IOLINK_CMD_DEVICE_OPERATE))
    {
        uint8_t pos = fake_iolink_device_queue_begin();

        g_device.transition_count++;
        g_device.link_state = FAKE_LINK_OPERATE;
        /* Figure A.5: a Type-0 WRITE is answered by the CKS octet alone. */
        g_device.rx_queue[pos] = 0x00U;
        g_device.rx_queue[pos] = iolink_checksum6(&g_device.rx_queue[pos], 1U);
        g_device.rx_len = (uint8_t)(pos + 1U);
        return (int)len;
    }

    if(len == IOLINK_M_SEQ_TYPE0_LEN)
    {
        /* Startup probe (spec T1): the first Type-0 frame after wake-up is a page-
           channel READ of a Direct Parameter octet. Answer it from the Direct
           Parameter page rather than treating it as ISDU traffic. */
        if((g_device.link_state == FAKE_LINK_STARTUP) &&
           ((data[0] & IOLINK_MC_RW_MASK) != 0U) &&
           ((data[0] & IOLINK_MC_COMM_CHANNEL_MASK) == 0x20U))
        {
            g_device.link_state = FAKE_LINK_PREOPERATE;
            fake_iolink_device_queue_type0(
                fake_iolink_device_direct_param_octet((uint8_t)(data[0] & IOLINK_MC_ADDR_MASK)));
            return (int)len;
        }

        if(data[0] == IOLINK_MC_TRANSITION_COMMAND)
        {
            g_device.transition_count++;
            g_device.link_state = FAKE_LINK_OPERATE;
            return (int)len;
        }

        if(g_device.link_state == FAKE_LINK_STARTUP)
        {
            g_device.link_state = FAKE_LINK_PREOPERATE;
        }
        fake_iolink_device_queue_type0(fake_iolink_device_next_response_od());
        return (int)len;
    }

    g_device.operate_cycle_count++;
    g_device.link_state = FAKE_LINK_OPERATE;
    fake_iolink_device_queue_operate_response((data[0] & IOLINK_MC_RW_MASK) != 0U);
    return (int)len;
}

static int fake_iolink_device_recv_byte(void* user, uint8_t* byte)
{
    (void)user;
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

void fake_iolink_device_set_event_memory(const uint8_t* memory, uint8_t len)
{
    uint8_t i;

    (void)memset(g_device.event_memory, 0, sizeof(g_device.event_memory));
    for(i = 0U; (i < len) && (i < sizeof(g_device.event_memory)); i++)
    {
        g_device.event_memory[i] = memory[i];
    }
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

void fake_iolink_device_keep_unread_responses(void)
{
    g_device.keep_unread_responses = true;
}

void fake_iolink_device_flush_rx(void)
{
    g_device.rx_len = 0U;
    g_device.rx_pos = 0U;
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
