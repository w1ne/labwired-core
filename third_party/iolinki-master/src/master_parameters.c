#include "master_internal.h"

#include <string.h>

static uint8_t iolink_master_decode_pd_descriptor(uint8_t descriptor)
{
    if(descriptor == 0U)
    {
        return 0U;
    }

    if((descriptor & 0x80U) != 0U)
    {
        return (uint8_t)((descriptor & 0x7FU) + 1U);
    }

    return (uint8_t)(descriptor / 8U);
}

static uint8_t iolink_master_mseq_capability_code(iolink_master_m_seq_type_t type)
{
    switch(type)
    {
    case IOLINK_MASTER_M_SEQ_TYPE_1_1:
    case IOLINK_MASTER_M_SEQ_TYPE_1_2:
        return 1U;
    case IOLINK_MASTER_M_SEQ_TYPE_1_V:
    case IOLINK_MASTER_M_SEQ_TYPE_2_V:
        return 5U;
    default:
        return 0U;
    }
}

static bool iolink_master_mseq_type_from_capability_code(uint8_t code,
                                                         bool isdu_supported,
                                                         uint8_t pd_in_len,
                                                         uint8_t pd_out_len,
                                                         iolink_master_m_seq_type_t* type)
{
    if(type == NULL)
    {
        return false;
    }

    switch(code)
    {
    case 0U:
        if((pd_in_len == 0U) && (pd_out_len == 0U))
        {
            *type = IOLINK_MASTER_M_SEQ_TYPE_0;
        }
        else if(isdu_supported)
        {
            *type = IOLINK_MASTER_M_SEQ_TYPE_2_2;
        }
        else
        {
            *type = IOLINK_MASTER_M_SEQ_TYPE_2_1;
        }
        return true;
    case 1U:
        *type = isdu_supported ? IOLINK_MASTER_M_SEQ_TYPE_1_2
                               : IOLINK_MASTER_M_SEQ_TYPE_1_1;
        return true;
    case 5U:
        *type = isdu_supported ? IOLINK_MASTER_M_SEQ_TYPE_2_V
                               : IOLINK_MASTER_M_SEQ_TYPE_1_V;
        return true;
    default:
        return false;
    }
}

int iolink_master_parse_direct_parameter_page1(const uint8_t* page,
                                               uint8_t len,
                                               iolink_master_device_info_t* info)
{
    if((page == NULL) || (info == NULL))
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    if(len < 16U)
    {
        return IOLINK_MASTER_PARAM_ERR_TOO_SHORT;
    }

    memset(info, 0, sizeof(*info));
    info->valid = true;
    info->min_cycle_time = page[0x02];
    info->mseq_capability = page[0x03];
    info->isdu_supported = ((page[0x03] & 0x01U) != 0U);
    info->operate_mseq_code = (uint8_t)((page[0x03] >> 1U) & 0x07U);
    info->preoperate_mseq_code = (uint8_t)((page[0x03] >> 4U) & 0x03U);
    info->revision_id = page[0x04];
    info->pd_in_descriptor = page[0x05];
    info->pd_out_descriptor = page[0x06];
    info->pd_in_len = iolink_master_decode_pd_descriptor(page[0x05]);
    info->pd_out_len = iolink_master_decode_pd_descriptor(page[0x06]);
    info->vendor_id = (uint16_t)(((uint16_t)page[0x07] << 8U) | page[0x08]);
    info->device_id = ((uint32_t)page[0x09] << 16U) | ((uint32_t)page[0x0A] << 8U) |
                      (uint32_t)page[0x0B];
    return IOLINK_MASTER_STATUS_OK;
}

int iolink_master_apply_direct_parameter_page1(iolink_master_port_t* port,
                                               const uint8_t* page,
                                               uint8_t len)
{
    iolink_master_port_state_t* state;

    if(port == NULL)
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    state = iolink_master_port_state(port);
    return iolink_master_parse_direct_parameter_page1(page, len, &state->device_info);
}

int iolink_master_get_device_info(const iolink_master_port_t* port,
                                  iolink_master_device_info_t* info)
{
    const iolink_master_port_state_t* state;

    if((port == NULL) || (info == NULL))
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    state = iolink_master_port_const_state(port);
    *info = state->device_info;
    if(!info->valid)
    {
        return IOLINK_MASTER_STATUS_PENDING;
    }

    return IOLINK_MASTER_STATUS_OK;
}

int iolink_master_validate_device_info(const iolink_master_port_t* port)
{
    const iolink_master_port_state_t* state;

    if(port == NULL)
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    state = iolink_master_port_const_state(port);
    return iolink_master_validate_config_against_device_info(&state->device_info, &state->config);
}

int iolink_master_validate_config_against_device_info(const iolink_master_device_info_t* info,
                                                      const iolink_master_config_t* config)
{
    if((info == NULL) || (config == NULL))
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    if(!info->valid)
    {
        return IOLINK_MASTER_STATUS_PENDING;
    }

    if((info->revision_id != 0x10U) && (info->revision_id != 0x11U))
    {
        return IOLINK_MASTER_PARAM_ERR_REVISION;
    }

    if(config->min_cycle_time < info->min_cycle_time)
    {
        return IOLINK_MASTER_PARAM_ERR_CYCLE_TIME;
    }

    if((config->pd_in_len != info->pd_in_len) || (config->pd_out_len != info->pd_out_len))
    {
        return IOLINK_MASTER_PARAM_ERR_PD_SIZE;
    }

    if(iolink_master_mseq_capability_code(config->m_seq_type) != info->operate_mseq_code)
    {
        return IOLINK_MASTER_PARAM_ERR_M_SEQUENCE;
    }

    return IOLINK_MASTER_STATUS_OK;
}

int iolink_master_select_config_from_device_info(const iolink_master_device_info_t* info,
                                                 iolink_master_config_t* config)
{
    iolink_master_m_seq_type_t m_seq_type;

    if((info == NULL) || (config == NULL))
    {
        return IOLINK_MASTER_ERR_INVALID_ARG;
    }

    if(!info->valid)
    {
        return IOLINK_MASTER_STATUS_PENDING;
    }

    if(!iolink_master_mseq_type_from_capability_code(info->operate_mseq_code,
                                                     info->isdu_supported,
                                                     info->pd_in_len,
                                                     info->pd_out_len,
                                                     &m_seq_type))
    {
        return IOLINK_MASTER_PARAM_ERR_M_SEQUENCE;
    }

    config->m_seq_type = m_seq_type;
    config->min_cycle_time = info->min_cycle_time;
    config->pd_in_len = info->pd_in_len;
    config->pd_out_len = info->pd_out_len;

    return IOLINK_MASTER_STATUS_OK;
}
