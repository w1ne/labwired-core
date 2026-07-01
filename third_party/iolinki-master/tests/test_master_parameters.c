#include <setjmp.h>
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#include <cmocka.h>

#include "../src/master_internal.h"

static const uint8_t g_page1[] = {
    0x00U, /* MasterCommand */
    0x00U, /* MasterCycleTime */
    0x0AU, /* MinCycleTime */
    0x0BU, /* M-sequenceCapability: ISDU + operate code 5 */
    0x11U, /* RevisionID */
    0x10U, /* ProcessDataIn: 16 bits */
    0x83U, /* ProcessDataOut: 4 octets */
    0x12U, /* VendorID MSB */
    0x34U, /* VendorID LSB */
    0x56U, /* DeviceID high */
    0x78U, /* DeviceID mid */
    0x9AU, /* DeviceID low */
    0x00U,
    0x00U,
    0x00U,
    0x00U,
};

static int null_phy_init(void* user)
{
    (void)user;
    return 0;
}

static const iolink_phy_api_t g_phy = {
    .init = null_phy_init,
};

static const iolink_master_config_t g_config = {
    .port_mode = IOLINK_MASTER_PORT_MODE_IOLINK,
    .m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_2_1,
    .baudrate = IOLINK_BAUDRATE_COM3,
    .min_cycle_time = 20U,
    .pd_in_len = 2U,
    .pd_out_len = 4U,
};

static void test_parse_direct_parameter_page1_decodes_standard_fields(void** state)
{
    iolink_master_device_info_t info;

    (void)state;

    memset(&info, 0, sizeof(info));

    assert_int_equal(iolink_master_parse_direct_parameter_page1(g_page1, sizeof(g_page1), &info),
                     0);
    assert_true(info.valid);
    assert_int_equal(info.min_cycle_time, 0x0AU);
    assert_int_equal(info.revision_id, 0x11U);
    assert_true(info.isdu_supported);
    assert_int_equal(info.operate_mseq_code, 5U);
    assert_int_equal(info.preoperate_mseq_code, 0U);
    assert_int_equal(info.pd_in_descriptor, 0x10U);
    assert_int_equal(info.pd_out_descriptor, 0x83U);
    assert_int_equal(info.pd_in_len, 2U);
    assert_int_equal(info.pd_out_len, 4U);
    assert_int_equal(info.vendor_id, 0x1234U);
    assert_int_equal(info.device_id, 0x56789AU);
}

static void test_parse_direct_parameter_page1_decodes_zero_and_small_bit_lengths(void** state)
{
    uint8_t page[16] = {0U};
    iolink_master_device_info_t info;

    (void)state;

    page[0x05] = 0x08U;
    page[0x06] = 0x00U;

    assert_int_equal(iolink_master_parse_direct_parameter_page1(page, sizeof(page), &info), 0);
    assert_int_equal(info.pd_in_len, 1U);
    assert_int_equal(info.pd_out_len, 0U);
}

static void test_parse_direct_parameter_page1_rejects_invalid_args(void** state)
{
    iolink_master_device_info_t info;

    (void)state;

    assert_int_equal(iolink_master_parse_direct_parameter_page1(NULL, sizeof(g_page1), &info),
                     -1);
    assert_int_equal(iolink_master_parse_direct_parameter_page1(g_page1, sizeof(g_page1), NULL),
                     -1);
    assert_int_equal(iolink_master_parse_direct_parameter_page1(g_page1,
                                                                (uint8_t)(sizeof(g_page1) - 1U),
                                                                &info),
                     -2);
}

static void test_apply_direct_parameter_page1_latches_info_on_port(void** state)
{
    iolink_master_port_t port;
    iolink_master_device_info_t info;

    (void)state;

    assert_int_equal(iolink_master_init(&port, &g_phy, &g_config), 0);
    assert_int_equal(iolink_master_apply_direct_parameter_page1(&port, g_page1, sizeof(g_page1)),
                     0);
    assert_int_equal(iolink_master_get_device_info(&port, &info), 0);
    assert_true(info.valid);
    assert_int_equal(info.vendor_id, 0x1234U);
    assert_int_equal(info.device_id, 0x56789AU);
    assert_int_equal(info.pd_in_len, 2U);
    assert_int_equal(info.pd_out_len, 4U);
}

static void test_get_device_info_rejects_invalid_or_unavailable_info(void** state)
{
    iolink_master_port_t port;
    iolink_master_device_info_t info;

    (void)state;

    assert_int_equal(iolink_master_get_device_info(NULL, &info), -1);
    assert_int_equal(iolink_master_get_device_info(&port, NULL), -1);

    assert_int_equal(iolink_master_init(&port, &g_phy, &g_config), 0);
    assert_int_equal(iolink_master_get_device_info(&port, &info), 1);
    assert_false(info.valid);
}

static void test_validate_device_info_accepts_matching_configuration(void** state)
{
    uint8_t page[16];
    iolink_master_port_t port;

    (void)state;

    memcpy(page, g_page1, sizeof(page));
    page[0x02] = 10U;
    page[0x03] = 0x01U; /* ISDU supported, operate M-sequence code 0. */

    assert_int_equal(iolink_master_init(&port, &g_phy, &g_config), 0);
    assert_int_equal(iolink_master_apply_direct_parameter_page1(&port, page, sizeof(page)), 0);
    assert_int_equal(iolink_master_validate_device_info(&port), 0);
}

static void test_validate_device_info_rejects_missing_or_invalid_info(void** state)
{
    iolink_master_port_t port;
    uint8_t page[16];

    (void)state;

    assert_int_equal(iolink_master_validate_device_info(NULL), -1);

    assert_int_equal(iolink_master_init(&port, &g_phy, &g_config), 0);
    assert_int_equal(iolink_master_validate_device_info(&port), 1);

    memcpy(page, g_page1, sizeof(page));
    page[0x04] = 0x22U;
    assert_int_equal(iolink_master_apply_direct_parameter_page1(&port, page, sizeof(page)), 0);
    assert_int_equal(iolink_master_validate_device_info(&port), -2);
}

static void test_validate_device_info_rejects_incompatible_cycle_pd_and_mseq(void** state)
{
    uint8_t page[16];
    iolink_master_port_t port;

    (void)state;

    memcpy(page, g_page1, sizeof(page));
    page[0x02] = 21U;
    assert_int_equal(iolink_master_init(&port, &g_phy, &g_config), 0);
    assert_int_equal(iolink_master_apply_direct_parameter_page1(&port, page, sizeof(page)), 0);
    assert_int_equal(iolink_master_validate_device_info(&port), -3);

    memcpy(page, g_page1, sizeof(page));
    page[0x05] = 0x18U;
    assert_int_equal(iolink_master_apply_direct_parameter_page1(&port, page, sizeof(page)), 0);
    assert_int_equal(iolink_master_validate_device_info(&port), -4);

    memcpy(page, g_page1, sizeof(page));
    page[0x03] = 0x03U; /* ISDU supported, operate M-sequence code 1. */
    assert_int_equal(iolink_master_apply_direct_parameter_page1(&port, page, sizeof(page)), 0);
    assert_int_equal(iolink_master_validate_device_info(&port), -5);
}

static void test_select_config_from_device_info_applies_capability_profile(void** state)
{
    iolink_master_device_info_t info;
    iolink_master_config_t config = {
        .port_mode = IOLINK_MASTER_PORT_MODE_IOLINK,
        .baudrate = IOLINK_BAUDRATE_COM2,
        .auto_baudrate = true,
        .validate_device_info = true,
    };

    (void)state;

    assert_int_equal(iolink_master_parse_direct_parameter_page1(g_page1, sizeof(g_page1), &info),
                     IOLINK_MASTER_STATUS_OK);
    assert_int_equal(iolink_master_select_config_from_device_info(&info, &config),
                     IOLINK_MASTER_STATUS_OK);
    assert_int_equal(config.port_mode, IOLINK_MASTER_PORT_MODE_IOLINK);
    assert_int_equal(config.baudrate, IOLINK_BAUDRATE_COM2);
    assert_true(config.auto_baudrate);
    assert_true(config.validate_device_info);
    assert_int_equal(config.min_cycle_time, 0x0AU);
    assert_int_equal(config.pd_in_len, 2U);
    assert_int_equal(config.pd_out_len, 4U);
    assert_int_equal(config.m_seq_type, IOLINK_MASTER_M_SEQ_TYPE_2_V);
}

static void test_select_config_from_device_info_maps_fixed_type2_profiles(void** state)
{
    uint8_t page[16];
    iolink_master_device_info_t info;
    iolink_master_config_t config = {
        .port_mode = IOLINK_MASTER_PORT_MODE_IOLINK,
        .baudrate = IOLINK_BAUDRATE_COM3,
    };

    (void)state;

    memcpy(page, g_page1, sizeof(page));
    page[0x03] = 0x00U; /* No ISDU, operate M-sequence code 0. */
    page[0x05] = 0x10U;
    page[0x06] = 0x10U;
    assert_int_equal(iolink_master_parse_direct_parameter_page1(page, sizeof(page), &info),
                     IOLINK_MASTER_STATUS_OK);
    assert_int_equal(iolink_master_select_config_from_device_info(&info, &config),
                     IOLINK_MASTER_STATUS_OK);
    assert_int_equal(config.m_seq_type, IOLINK_MASTER_M_SEQ_TYPE_2_1);
    assert_int_equal(config.pd_in_len, 2U);
    assert_int_equal(config.pd_out_len, 2U);
    assert_int_equal(iolink_master_validate_config_against_device_info(&info, &config),
                     IOLINK_MASTER_STATUS_OK);

    page[0x03] = 0x01U; /* ISDU supported, operate M-sequence code 0. */
    assert_int_equal(iolink_master_parse_direct_parameter_page1(page, sizeof(page), &info),
                     IOLINK_MASTER_STATUS_OK);
    assert_int_equal(iolink_master_select_config_from_device_info(&info, &config),
                     IOLINK_MASTER_STATUS_OK);
    assert_int_equal(config.m_seq_type, IOLINK_MASTER_M_SEQ_TYPE_2_2);
    assert_int_equal(iolink_master_validate_config_against_device_info(&info, &config),
                     IOLINK_MASTER_STATUS_OK);
}

static void test_select_config_from_device_info_maps_all_public_mseq_profiles(void** state)
{
    static const struct
    {
        uint8_t capability;
        uint8_t pd_in_descriptor;
        uint8_t pd_out_descriptor;
        iolink_master_m_seq_type_t expected_type;
        uint8_t expected_pd_in_len;
        uint8_t expected_pd_out_len;
    } cases[] = {
        {0x00U, 0x00U, 0x00U, IOLINK_MASTER_M_SEQ_TYPE_0, 0U, 0U},
        {0x00U, 0x08U, 0x08U, IOLINK_MASTER_M_SEQ_TYPE_2_1, 1U, 1U},
        {0x01U, 0x10U, 0x10U, IOLINK_MASTER_M_SEQ_TYPE_2_2, 2U, 2U},
        {0x02U, 0x08U, 0x08U, IOLINK_MASTER_M_SEQ_TYPE_1_1, 1U, 1U},
        {0x03U, 0x10U, 0x10U, IOLINK_MASTER_M_SEQ_TYPE_1_2, 2U, 2U},
        {0x0AU, 0x83U, 0x83U, IOLINK_MASTER_M_SEQ_TYPE_1_V, 4U, 4U},
        {0x0BU, 0x84U, 0x84U, IOLINK_MASTER_M_SEQ_TYPE_2_V, 5U, 5U},
    };
    uint8_t page[16];
    iolink_master_device_info_t info;
    iolink_master_config_t config = {
        .port_mode = IOLINK_MASTER_PORT_MODE_IOLINK,
        .baudrate = IOLINK_BAUDRATE_COM3,
    };
    size_t i;

    (void)state;

    for(i = 0U; i < (sizeof(cases) / sizeof(cases[0])); i++)
    {
        memcpy(page, g_page1, sizeof(page));
        page[0x03] = cases[i].capability;
        page[0x05] = cases[i].pd_in_descriptor;
        page[0x06] = cases[i].pd_out_descriptor;

        assert_int_equal(iolink_master_parse_direct_parameter_page1(page, sizeof(page), &info),
                         IOLINK_MASTER_STATUS_OK);
        assert_int_equal(iolink_master_select_config_from_device_info(&info, &config),
                         IOLINK_MASTER_STATUS_OK);
        assert_int_equal(config.m_seq_type, cases[i].expected_type);
        assert_int_equal(config.pd_in_len, cases[i].expected_pd_in_len);
        assert_int_equal(config.pd_out_len, cases[i].expected_pd_out_len);
        assert_int_equal(iolink_master_validate_config_against_device_info(&info, &config),
                         IOLINK_MASTER_STATUS_OK);
    }
}

static void test_validate_config_against_device_info_rejects_incompatible_request(void** state)
{
    iolink_master_device_info_t info;
    iolink_master_config_t config = g_config;

    (void)state;

    assert_int_equal(iolink_master_parse_direct_parameter_page1(g_page1, sizeof(g_page1), &info),
                     IOLINK_MASTER_STATUS_OK);

    config.m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_2_V;
    assert_int_equal(iolink_master_validate_config_against_device_info(&info, &config),
                     IOLINK_MASTER_STATUS_OK);

    config.min_cycle_time = 9U;
    assert_int_equal(iolink_master_validate_config_against_device_info(&info, &config),
                     IOLINK_MASTER_PARAM_ERR_CYCLE_TIME);

    config = g_config;
    config.pd_out_len = 3U;
    assert_int_equal(iolink_master_validate_config_against_device_info(&info, &config),
                     IOLINK_MASTER_PARAM_ERR_PD_SIZE);

    config = g_config;
    config.m_seq_type = IOLINK_MASTER_M_SEQ_TYPE_1_1;
    assert_int_equal(iolink_master_validate_config_against_device_info(&info, &config),
                     IOLINK_MASTER_PARAM_ERR_M_SEQUENCE);
}

static void test_select_config_from_device_info_rejects_invalid_inputs(void** state)
{
    iolink_master_device_info_t info = {0};
    iolink_master_config_t config = g_config;

    (void)state;

    assert_int_equal(iolink_master_select_config_from_device_info(NULL, &config),
                     IOLINK_MASTER_ERR_INVALID_ARG);
    assert_int_equal(iolink_master_select_config_from_device_info(&info, NULL),
                     IOLINK_MASTER_ERR_INVALID_ARG);
    assert_int_equal(iolink_master_select_config_from_device_info(&info, &config),
                     IOLINK_MASTER_STATUS_PENDING);

    info.valid = true;
    info.operate_mseq_code = 7U;
    assert_int_equal(iolink_master_select_config_from_device_info(&info, &config),
                     IOLINK_MASTER_PARAM_ERR_M_SEQUENCE);
}

int main(void)
{
    const struct CMUnitTest tests[] = {
        cmocka_unit_test(test_parse_direct_parameter_page1_decodes_standard_fields),
        cmocka_unit_test(test_parse_direct_parameter_page1_decodes_zero_and_small_bit_lengths),
        cmocka_unit_test(test_parse_direct_parameter_page1_rejects_invalid_args),
        cmocka_unit_test(test_apply_direct_parameter_page1_latches_info_on_port),
        cmocka_unit_test(test_get_device_info_rejects_invalid_or_unavailable_info),
        cmocka_unit_test(test_validate_device_info_accepts_matching_configuration),
        cmocka_unit_test(test_validate_device_info_rejects_missing_or_invalid_info),
        cmocka_unit_test(test_validate_device_info_rejects_incompatible_cycle_pd_and_mseq),
        cmocka_unit_test(test_select_config_from_device_info_applies_capability_profile),
        cmocka_unit_test(test_select_config_from_device_info_maps_fixed_type2_profiles),
        cmocka_unit_test(test_select_config_from_device_info_maps_all_public_mseq_profiles),
        cmocka_unit_test(test_validate_config_against_device_info_rejects_incompatible_request),
        cmocka_unit_test(test_select_config_from_device_info_rejects_invalid_inputs),
    };

    return cmocka_run_group_tests(tests, NULL, NULL);
}
