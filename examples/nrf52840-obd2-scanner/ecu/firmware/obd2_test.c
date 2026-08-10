#include "obd2.h"

#include <assert.h>
#include <stdio.h>
#include <string.h>

static obd2_frame_t req(uint32_t id, const uint8_t bytes[8])
{
    obd2_frame_t f = {.id = id, .dlc = 8};
    memcpy(f.data, bytes, 8);
    return f;
}

static void expect(obd2_ecu_t *ecu, const uint8_t in[8], const uint8_t out[8])
{
    obd2_frame_t request = req(0x7DF, in), response;
    assert(obd2_process(ecu, &request, 10, &response) == OBD2_FRAME_READY);
    assert(response.id == 0x7E8 && response.dlc == 8);
    assert(memcmp(response.data, out, 8) == 0);
}

int main(void)
{
    assert(!obd2_vin_expired(UINT16_MAX - 500u, UINT16_MAX - 500u));
    assert(!obd2_vin_expired(498u, UINT16_MAX - 500u));
    assert(obd2_vin_expired(499u, UINT16_MAX - 500u));
    assert(obd2_tx_status(0u) == OBD2_TX_PENDING);
    assert(obd2_tx_status(1u << 0) == OBD2_TX_FAILED);
    assert(obd2_tx_status(1u << 1) == OBD2_TX_OK);
    assert(obd2_tx_status(1u << 2) == OBD2_TX_FAILED);
    assert(obd2_tx_status(1u << 3) == OBD2_TX_FAILED);

    obd2_ecu_t ecu;
    obd2_init(&ecu);

    const uint8_t pid00_req[8] = {2, 1, 0, 0, 0, 0, 0, 0};
    const uint8_t pid00_rsp[8] = {6, 0x41, 0, 0x08, 0x18, 0, 0, 0};
    expect(&ecu, pid00_req, pid00_rsp);
    const uint8_t rpm_req[8] = {2, 1, 0x0C, 0, 0, 0, 0, 0};
    const uint8_t rpm_rsp[8] = {4, 0x41, 0x0C, 0x2E, 0xE0, 0, 0, 0};
    expect(&ecu, rpm_req, rpm_rsp);
    const uint8_t speed_req[8] = {2, 1, 0x0D, 0, 0, 0, 0, 0};
    const uint8_t speed_rsp[8] = {3, 0x41, 0x0D, 88, 0, 0, 0, 0};
    expect(&ecu, speed_req, speed_rsp);
    const uint8_t temp_req[8] = {2, 1, 5, 0, 0, 0, 0, 0};
    const uint8_t temp_rsp[8] = {3, 0x41, 5, 130, 0, 0, 0, 0};
    expect(&ecu, temp_req, temp_rsp);

    const uint8_t dtc_req[8] = {1, 3, 0, 0, 0, 0, 0, 0};
    const uint8_t dtc_rsp[8] = {5, 0x43, 0x01, 0x33, 0xC1, 0x23, 0, 0};
    expect(&ecu, dtc_req, dtc_rsp);
    const uint8_t clear_req[8] = {1, 4, 0, 0, 0, 0, 0, 0};
    const uint8_t clear_rsp[8] = {1, 0x44, 0, 0, 0, 0, 0, 0};
    expect(&ecu, clear_req, clear_rsp);
    const uint8_t empty_rsp[8] = {1, 0x43, 0, 0, 0, 0, 0, 0};
    expect(&ecu, dtc_req, empty_rsp);

    const uint8_t vin_req[8] = {2, 9, 2, 0, 0, 0, 0, 0};
    const uint8_t vin_ff[8] = {0x10, 0x14, 0x49, 2, 1, 'L', 'W', 'O'};
    expect(&ecu, vin_req, vin_ff);
    obd2_frame_t response;
    assert(obd2_poll(&ecu, 11, &response) == OBD2_NO_FRAME);
    const uint8_t wait_fc_bytes[8] = {0x31, 0, 0, 0, 0, 0, 0, 0};
    obd2_frame_t wait_fc = req(0x7E0, wait_fc_bytes);
    assert(obd2_process(&ecu, &wait_fc, 12, &response) == OBD2_NO_FRAME);
    assert(obd2_poll(&ecu, 12, &response) == OBD2_NO_FRAME);
    assert(obd2_poll(&ecu, 1010, &response) == OBD2_NO_FRAME);
    assert(ecu.vin_transfer_state == 0);

    expect(&ecu, vin_req, vin_ff);
    const uint8_t fc_bytes[8] = {0x30, 0, 0, 0, 0, 0, 0, 0};
    obd2_frame_t fc = req(0x7E0, fc_bytes);
    const uint8_t cf1[8] = {0x21, 'B', 'D', '2', 'S', 'I', 'M', '0'};
    assert(obd2_process(&ecu, &fc, 20, &response) == OBD2_FRAME_READY);
    assert(memcmp(response.data, cf1, 8) == 0);
    const uint8_t cf2[8] = {0x22, '0', '0', '0', '0', '0', '0', '1'};
    assert(obd2_poll(&ecu, 21, &response) == OBD2_FRAME_READY);
    assert(memcmp(response.data, cf2, 8) == 0);
    assert(obd2_poll(&ecu, 22, &response) == OBD2_NO_FRAME);

    /* BS=1 permits CF1 only; a second CTS is required for CF2. */
    expect(&ecu, vin_req, vin_ff);
    const uint8_t bs1_bytes[8] = {0x30, 1, 0, 0, 0, 0, 0, 0};
    obd2_frame_t bs1 = req(0x7E0, bs1_bytes);
    assert(obd2_process(&ecu, &bs1, 30, &response) == OBD2_FRAME_READY);
    assert(memcmp(response.data, cf1, 8) == 0);
    assert(obd2_poll(&ecu, 31, &response) == OBD2_NO_FRAME);
    assert(obd2_process(&ecu, &fc, 32, &response) == OBD2_FRAME_READY);
    assert(memcmp(response.data, cf2, 8) == 0);

    /* STmin gates CF1 and the interval from CF1 to CF2. */
    expect(&ecu, vin_req, vin_ff);
    const uint8_t stmin_bytes[8] = {0x30, 0, 5, 0, 0, 0, 0, 0};
    obd2_frame_t stmin = req(0x7E0, stmin_bytes);
    assert(obd2_process(&ecu, &stmin, 40, &response) == OBD2_NO_FRAME);
    assert(obd2_poll(&ecu, 44, &response) == OBD2_NO_FRAME);
    assert(obd2_poll(&ecu, 45, &response) == OBD2_FRAME_READY);
    assert(memcmp(response.data, cf1, 8) == 0);
    assert(obd2_poll(&ecu, 49, &response) == OBD2_NO_FRAME);
    assert(obd2_poll(&ecu, 50, &response) == OBD2_FRAME_READY);
    assert(memcmp(response.data, cf2, 8) == 0);

    /* 100us STmin encodings round conservatively to one 1kHz timer tick. */
    expect(&ecu, vin_req, vin_ff);
    const uint8_t fine_stmin_bytes[8] = {0x30, 0, 0xF1, 0, 0, 0, 0, 0};
    obd2_frame_t fine_stmin = req(0x7E0, fine_stmin_bytes);
    assert(obd2_process(&ecu, &fine_stmin, 60, &response) == OBD2_NO_FRAME);
    assert(obd2_poll(&ecu, 60, &response) == OBD2_NO_FRAME);
    assert(obd2_poll(&ecu, 61, &response) == OBD2_FRAME_READY);

    /* Reserved STmin and OVFLW terminate the transfer as malformed. */
    expect(&ecu, vin_req, vin_ff);
    const uint8_t reserved_bytes[8] = {0x30, 0, 0x80, 0, 0, 0, 0, 0};
    obd2_frame_t reserved = req(0x7E0, reserved_bytes);
    assert(obd2_process(&ecu, &reserved, 70, &response) == OBD2_MALFORMED);
    assert(ecu.vin_transfer_state == 0);
    expect(&ecu, vin_req, vin_ff);
    const uint8_t overflow_bytes[8] = {0x32, 0, 0, 0, 0, 0, 0, 0};
    obd2_frame_t overflow = req(0x7E0, overflow_bytes);
    assert(obd2_process(&ecu, &overflow, 80, &response) == OBD2_MALFORMED);
    assert(ecu.vin_transfer_state == 0);

    expect(&ecu, vin_req, vin_ff);
    assert(obd2_poll(&ecu, 1011, &response) == OBD2_NO_FRAME);
    assert(ecu.vin_transfer_state == 0);

    const uint8_t bad_pid[8] = {2, 1, 0x99, 0, 0, 0, 0, 0};
    const uint8_t bad_pid_rsp[8] = {3, 0x7F, 1, 0x12, 0, 0, 0, 0};
    expect(&ecu, bad_pid, bad_pid_rsp);
    const uint8_t bad_service[8] = {1, 0x22, 0, 0, 0, 0, 0, 0};
    const uint8_t bad_service_rsp[8] = {3, 0x7F, 0x22, 0x11, 0, 0, 0, 0};
    expect(&ecu, bad_service, bad_service_rsp);

    obd2_frame_t malformed = req(0x7DF, pid00_req);
    malformed.dlc = 2;
    assert(obd2_process(&ecu, &malformed, 0, &response) == OBD2_MALFORMED);
    malformed = req(0x7DF, pid00_req);
    malformed.data[0] = 7;
    assert(obd2_process(&ecu, &malformed, 0, &response) == OBD2_MALFORMED);
    malformed = req(0x123, pid00_req);
    assert(obd2_process(&ecu, &malformed, 0, &response) == OBD2_NO_FRAME);

    puts("obd2 protocol tests passed");
    return 0;
}
