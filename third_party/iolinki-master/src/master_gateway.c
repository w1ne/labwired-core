#include "iolinki_master/master.h"

#include <stddef.h>
#include <stdint.h>
#include <string.h>

static int gateway_append(char* out, size_t out_len, size_t* used, char ch)
{
    if((*used + 2U) > out_len)
    {
        return -1;
    }

    out[*used] = ch;
    *used += 1U;
    out[*used] = '\0';
    return 0;
}

static int gateway_append_str(char* out, size_t out_len, size_t* used, const char* text)
{
    size_t i;

    for(i = 0U; text[i] != '\0'; i++)
    {
        if(gateway_append(out, out_len, used, text[i]) != 0)
        {
            return -1;
        }
    }

    return 0;
}

static int gateway_append_u8_dec(char* out, size_t out_len, size_t* used, uint8_t value)
{
    char digits[3];
    uint8_t count = 0U;
    uint8_t n = value;

    do
    {
        digits[count] = (char)('0' + (n % 10U));
        count++;
        n = (uint8_t)(n / 10U);
    } while(n != 0U);

    while(count > 0U)
    {
        count--;
        if(gateway_append(out, out_len, used, digits[count]) != 0)
        {
            return -1;
        }
    }

    return 0;
}

static int gateway_append_hex(char* out, size_t out_len, size_t* used, uint32_t value, uint8_t width)
{
    static const char k_hex[] = "0123456789abcdef";
    uint8_t i;

    for(i = width; i > 0U; i--)
    {
        uint8_t shift = (uint8_t)((i - 1U) * 4U);
        if(gateway_append(out, out_len, used, k_hex[(value >> shift) & 0x0FU]) != 0)
        {
            return -1;
        }
    }

    return 0;
}

int iolink_master_format_gateway_line(uint8_t port_index,
                                      const iolink_master_device_info_t* info,
                                      const uint8_t* pd_in,
                                      uint8_t pd_in_len,
                                      char* out,
                                      size_t out_len)
{
    size_t used = 0U;
    uint8_t i;
    char line[IOLINK_MASTER_GATEWAY_LINE_MAX];

    if((info == NULL) || (out == NULL) || (out_len == 0U) || !info->valid ||
       (pd_in_len > IOLINK_PD_IN_MAX_SIZE) || ((pd_in_len > 0U) && (pd_in == NULL)))
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    line[0] = '\0';
    if((gateway_append_str(line, sizeof(line), &used, "iolinki-gw/1 ") != 0) ||
       (gateway_append_u8_dec(line, sizeof(line), &used, port_index) != 0) ||
       (gateway_append(line, sizeof(line), &used, ' ') != 0) ||
       (gateway_append_hex(line, sizeof(line), &used, info->vendor_id, 4U) != 0) ||
       (gateway_append(line, sizeof(line), &used, ' ') != 0) ||
       (gateway_append_hex(line, sizeof(line), &used, info->device_id, 8U) != 0) ||
       (gateway_append(line, sizeof(line), &used, ' ') != 0))
    {
        return IOLINK_MASTER_ERR_BUFFER_TOO_SMALL;
    }

    if(pd_in_len == 0U)
    {
        if(gateway_append(line, sizeof(line), &used, '-') != 0)
        {
            return IOLINK_MASTER_ERR_BUFFER_TOO_SMALL;
        }
    }
    else
    {
        for(i = 0U; i < pd_in_len; i++)
        {
            if(gateway_append_hex(line, sizeof(line), &used, pd_in[i], 2U) != 0)
            {
                return IOLINK_MASTER_ERR_BUFFER_TOO_SMALL;
            }
        }
    }

    if(gateway_append(line, sizeof(line), &used, '\n') != 0)
    {
        return IOLINK_MASTER_ERR_BUFFER_TOO_SMALL;
    }

    if((used + 1U) > out_len)
    {
        return IOLINK_MASTER_ERR_BUFFER_TOO_SMALL;
    }

    memcpy(out, line, used + 1U);
    return IOLINK_MASTER_STATUS_OK;
}

int iolink_master_write_gateway_line(const iolink_master_port_t* port,
                                     uint8_t port_index,
                                     char* out,
                                     size_t out_len)
{
    iolink_master_device_info_t info;
    uint8_t pd[IOLINK_PD_IN_MAX_SIZE];
    uint8_t pd_len = 0U;
    int ret;

    if((port == NULL) || (out == NULL) || (out_len == 0U))
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    ret = iolink_master_get_device_info(port, &info);
    if(ret != IOLINK_MASTER_STATUS_OK)
    {
        return ret;
    }

    ret = iolink_master_get_pd_in(port, pd, (uint8_t)sizeof(pd), &pd_len);
    if(ret == IOLINK_MASTER_STATUS_PENDING)
    {
        return iolink_master_format_gateway_line(port_index, &info, NULL, 0U, out, out_len);
    }
    if(ret != IOLINK_MASTER_STATUS_OK)
    {
        return ret;
    }

    return iolink_master_format_gateway_line(port_index, &info, pd, pd_len, out, out_len);
}
