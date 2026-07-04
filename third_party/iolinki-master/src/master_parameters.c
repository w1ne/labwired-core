#include "master_internal.h"

#include "iolinki/protocol.h"

#include <string.h>

uint16_t iolink_master_decode_min_cycle_time_100us(uint8_t octet)
{
    uint8_t time_base =
        (uint8_t)((octet >> IOLINK_MASTER_MIN_CYCLE_BASE_SHIFT) & IOLINK_MASTER_MIN_CYCLE_BASE_MASK);
    uint8_t multiplier = (uint8_t)(octet & IOLINK_MASTER_MIN_CYCLE_MULT_MASK);

    switch(time_base)
    {
    case 0U:
        /* 0.1 ms base: octet value is already the 100us count (0..6.3 ms). */
        return multiplier;
    case 1U:
        /* 0.4 ms base, offset 6.4 ms: 64 + n*4 (100us units), 6.4..31.6 ms. */
        return (uint16_t)(64U + ((uint16_t)multiplier * 4U));
    case 2U:
        /* 1.6 ms base, offset 32.0 ms: 320 + n*16 (100us units), 32.0..132.8 ms. */
        return (uint16_t)(320U + ((uint16_t)multiplier * 16U));
    default:
        /* Reserved base 11: fall back to the raw octet rather than inventing timing. */
        return octet;
    }
}

uint8_t iolink_master_encode_master_command(bool read,
                                            iolink_master_mc_channel_t channel,
                                            uint8_t address)
{
    uint8_t mc = (uint8_t)(address & IOLINK_MC_ADDR_MASK);

    mc = (uint8_t)(mc | ((uint8_t)((uint8_t)channel << IOLINK_MASTER_MC_COMM_CHANNEL_SHIFT) &
                         IOLINK_MC_COMM_CHANNEL_MASK));
    if(read)
    {
        mc = (uint8_t)(mc | IOLINK_MC_RW_MASK);
    }

    return mc;
}

bool iolink_master_mc_is_read(uint8_t mc)
{
    return (mc & IOLINK_MC_RW_MASK) != 0U;
}

iolink_master_mc_channel_t iolink_master_mc_channel(uint8_t mc)
{
    return (iolink_master_mc_channel_t)((mc & IOLINK_MC_COMM_CHANNEL_MASK) >>
                                        IOLINK_MASTER_MC_COMM_CHANNEL_SHIFT);
}

uint8_t iolink_master_mc_address(uint8_t mc)
{
    return (uint8_t)(mc & IOLINK_MC_ADDR_MASK);
}

static uint8_t iolink_master_decode_pd_descriptor(uint8_t descriptor)
{
    /*
     * ProcessData descriptor (Direct Parameter Page 1, Figure B.5 / Table B.6):
     *   bit 7   = BYTE  (length unit: 1 = octets, 0 = bits)
     *   bit 6   = SIO   (switching signal available in SIO mode)
     *   bit 5   = reserved
     *   bits 0-4 = Length
     * Only bits 0-4 carry the length, so the SIO/reserved bits must be masked
     * off before decoding (a SIO-capable device sets bit 6 legally).
     */
    uint8_t length = (uint8_t)(descriptor & IOLINK_MASTER_PD_DESC_LENGTH_MASK);

    if((descriptor & IOLINK_MASTER_PD_DESC_BYTE_BIT) != 0U)
    {
        /* BYTE = 1: octets. Table B.6 maps Length code n to (n + 1) octets. */
        return (uint8_t)(length + 1U);
    }

    /* BYTE = 0: Length is in bits (0..16); round up to whole octets. */
    return (uint8_t)((length + (IOLINK_MASTER_PD_DESC_BITS_PER_OCTET - 1U)) /
                     IOLINK_MASTER_PD_DESC_BITS_PER_OCTET);
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

    if(len < IOLINK_MASTER_DPP1_LEN)
    {
        return IOLINK_MASTER_PARAM_ERR_TOO_SHORT;
    }

    (void)memset(info, 0, sizeof(*info));
    info->valid = true;
    info->min_cycle_time = page[IOLINK_MASTER_DPP1_OFF_MIN_CYCLE_TIME];
    info->min_cycle_time_100us =
        iolink_master_decode_min_cycle_time_100us(page[IOLINK_MASTER_DPP1_OFF_MIN_CYCLE_TIME]);
    info->mseq_capability = page[IOLINK_MASTER_DPP1_OFF_MSEQ_CAPABILITY];
    info->isdu_supported =
        ((page[IOLINK_MASTER_DPP1_OFF_MSEQ_CAPABILITY] & IOLINK_MASTER_MSEQ_CAP_ISDU_BIT) != 0U);
    info->operate_mseq_code =
        (uint8_t)((page[IOLINK_MASTER_DPP1_OFF_MSEQ_CAPABILITY] >>
                   IOLINK_MASTER_MSEQ_CAP_OPERATE_SHIFT) & IOLINK_MASTER_MSEQ_CAP_OPERATE_MASK);
    info->preoperate_mseq_code =
        (uint8_t)((page[IOLINK_MASTER_DPP1_OFF_MSEQ_CAPABILITY] >>
                   IOLINK_MASTER_MSEQ_CAP_PREOP_SHIFT) & IOLINK_MASTER_MSEQ_CAP_PREOP_MASK);
    info->revision_id = page[IOLINK_MASTER_DPP1_OFF_REVISION_ID];
    info->pd_in_descriptor = page[IOLINK_MASTER_DPP1_OFF_PD_IN_DESC];
    info->pd_out_descriptor = page[IOLINK_MASTER_DPP1_OFF_PD_OUT_DESC];
    info->pd_in_len = iolink_master_decode_pd_descriptor(page[IOLINK_MASTER_DPP1_OFF_PD_IN_DESC]);
    info->pd_out_len = iolink_master_decode_pd_descriptor(page[IOLINK_MASTER_DPP1_OFF_PD_OUT_DESC]);
    info->vendor_id =
        (uint16_t)(((uint16_t)page[IOLINK_MASTER_DPP1_OFF_VENDOR_ID_HI] << 8U) |
                   page[IOLINK_MASTER_DPP1_OFF_VENDOR_ID_LO]);
    info->device_id = ((uint32_t)page[IOLINK_MASTER_DPP1_OFF_DEVICE_ID_HI] << 16U) |
                      ((uint32_t)page[IOLINK_MASTER_DPP1_OFF_DEVICE_ID_MID] << 8U) |
                      (uint32_t)page[IOLINK_MASTER_DPP1_OFF_DEVICE_ID_LO];
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

    if((info->revision_id != IOLINK_MASTER_REVISION_1_0) &&
       (info->revision_id != IOLINK_MASTER_REVISION_1_1))
    {
        return IOLINK_MASTER_PARAM_ERR_REVISION;
    }

    /*
     * The master's configured cycle time (raw 100us count) must be at least the
     * device's decoded MinCycleTime. Comparing decoded 100us values is what makes
     * devices that report a non-zero time base (0.4 / 1.6 ms) time correctly;
     * for the common 0.1 ms base the decoded value equals the raw octet.
     */
    if((uint16_t)config->min_cycle_time < info->min_cycle_time_100us)
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

    /*
     * Device identity check. Any inspection level other than NO_CHECK rejects a
     * device whose VendorID/DeviceID differ from the configured expected values.
     * The SerialNumber leg that distinguishes IDENTICAL from TYPE_COMP is a
     * documented follow-up (it is not carried in Direct Parameter Page 1).
     */
    if(config->inspection_level != IOLINK_MASTER_INSPECTION_NO_CHECK)
    {
        if(info->vendor_id != config->expected_vendor_id)
        {
            return IOLINK_MASTER_PARAM_ERR_VENDOR_ID;
        }
        if(info->device_id != config->expected_device_id)
        {
            return IOLINK_MASTER_PARAM_ERR_DEVICE_ID;
        }
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
    /*
     * Adopt the device's decoded MinCycleTime as the port cycle time (100us
     * units). Clamp to the uint8 config field: a device whose minimum exceeds
     * 25.5 ms cannot be paced by this field and will subsequently fail
     * validation, which is the honest outcome rather than silently wrapping.
     */
    config->min_cycle_time = (info->min_cycle_time_100us > (uint16_t)UINT8_MAX)
                                 ? UINT8_MAX
                                 : (uint8_t)info->min_cycle_time_100us;
    config->pd_in_len = info->pd_in_len;
    config->pd_out_len = info->pd_out_len;

    return IOLINK_MASTER_STATUS_OK;
}
