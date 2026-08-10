#include "obd2.h"

#include <stddef.h>

#define REQUEST_ID 0x7DFu
#define FLOW_CONTROL_ID 0x7E0u
#define RESPONSE_ID 0x7E8u
#define VIN_TIMEOUT_MS 1000u
#define VIN_WAIT_FC 1u
#define VIN_CF1_PENDING 2u
#define VIN_WAIT_NEXT_FC 3u
#define VIN_CF2_PENDING 4u

static void clear_frame(obd2_frame_t *frame)
{
    frame->id = RESPONSE_ID;
    frame->dlc = 8u;
    for (uint8_t i = 0; i < 8u; ++i) frame->data[i] = 0u;
}

static int negative(obd2_frame_t *response, uint8_t service, uint8_t nrc)
{
    clear_frame(response);
    response->data[0] = 3u;
    response->data[1] = 0x7Fu;
    response->data[2] = service;
    response->data[3] = nrc;
    return OBD2_FRAME_READY;
}

void obd2_init(obd2_ecu_t *ecu)
{
    ecu->dtc_count = 2u;
    ecu->vin_transfer_state = 0u;
    ecu->vin_block_remaining = 0u;
    ecu->vin_stmin_ms = 0u;
    ecu->vin_started_ms = 0u;
    ecu->vin_next_tx_ms = 0u;
}

bool obd2_vin_expired(uint32_t now_ms, uint32_t started_ms)
{
    return (uint16_t)(now_ms - started_ms) >= VIN_TIMEOUT_MS;
}

int obd2_tx_status(uint32_t tsr)
{
    if ((tsr & (1u << 1)) != 0u) return OBD2_TX_OK;
    if ((tsr & ((1u << 2) | (1u << 3))) != 0u) return OBD2_TX_FAILED;
    if ((tsr & (1u << 0)) != 0u) return OBD2_TX_FAILED;
    return OBD2_TX_PENDING;
}

static int mode01(const obd2_frame_t *request, obd2_frame_t *response)
{
    if (request->data[0] != 2u) return OBD2_MALFORMED;
    clear_frame(response);
    response->data[1] = 0x41u;
    response->data[2] = request->data[2];
    switch (request->data[2]) {
    case 0x00u:
        response->data[0] = 6u;
        /* SAE J1979: PID 01 is bit 31, so 05/0C/0D => 0x08180000. */
        response->data[3] = 0x08u;
        response->data[4] = 0x18u;
        return OBD2_FRAME_READY;
    case 0x05u:
        response->data[0] = 3u;
        response->data[3] = 130u; /* 90 C + 40 */
        return OBD2_FRAME_READY;
    case 0x0Cu:
        response->data[0] = 4u;
        response->data[3] = 0x2Eu;
        response->data[4] = 0xE0u; /* 3000 RPM * 4 */
        return OBD2_FRAME_READY;
    case 0x0Du:
        response->data[0] = 3u;
        response->data[3] = 88u;
        return OBD2_FRAME_READY;
    default:
        return negative(response, 0x01u, 0x12u);
    }
}

static int mode03(const obd2_ecu_t *ecu, const obd2_frame_t *request,
                  obd2_frame_t *response)
{
    if (request->data[0] != 1u) return OBD2_MALFORMED;
    clear_frame(response);
    response->data[1] = 0x43u;
    if (ecu->dtc_count == 0u) {
        response->data[0] = 1u;
    } else {
        response->data[0] = 5u;
        response->data[2] = 0x01u;
        response->data[3] = 0x33u;
        response->data[4] = 0xC1u;
        response->data[5] = 0x23u;
    }
    return OBD2_FRAME_READY;
}

static int mode04(obd2_ecu_t *ecu, const obd2_frame_t *request,
                  obd2_frame_t *response)
{
    if (request->data[0] != 1u) return OBD2_MALFORMED;
    clear_frame(response);
    response->data[0] = 1u;
    response->data[1] = 0x44u;
    ecu->dtc_count = 0u;
    return OBD2_FRAME_READY;
}

static int mode09(obd2_ecu_t *ecu, const obd2_frame_t *request, uint32_t now_ms,
                  obd2_frame_t *response)
{
    if (request->data[0] != 2u) return OBD2_MALFORMED;
    if (request->data[2] != 0x02u) return negative(response, 0x09u, 0x12u);
    clear_frame(response);
    const uint8_t ff[8] = {0x10u, 0x14u, 0x49u, 0x02u, 0x01u, 'L', 'W', 'O'};
    for (uint8_t i = 0; i < 8u; ++i) response->data[i] = ff[i];
    ecu->vin_transfer_state = VIN_WAIT_FC;
    ecu->vin_started_ms = now_ms;
    return OBD2_FRAME_READY;
}

static bool time_reached(uint32_t now_ms, uint32_t due_ms)
{
    return (uint16_t)(now_ms - due_ms) < 0x8000u;
}

static int emit_vin_cf(obd2_ecu_t *ecu, uint32_t now_ms,
                       obd2_frame_t *response)
{
    clear_frame(response);
    if (ecu->vin_transfer_state == VIN_CF1_PENDING) {
        const uint8_t cf1[8] = {0x21u, 'B', 'D', '2', 'S', 'I', 'M', '0'};
        for (uint8_t i = 0; i < 8u; ++i) response->data[i] = cf1[i];
        if (ecu->vin_block_remaining == 1u) {
            ecu->vin_block_remaining = 0u;
            ecu->vin_transfer_state = VIN_WAIT_NEXT_FC;
        } else {
            if (ecu->vin_block_remaining > 1u) --ecu->vin_block_remaining;
            ecu->vin_transfer_state = VIN_CF2_PENDING;
            ecu->vin_next_tx_ms = now_ms + ecu->vin_stmin_ms;
        }
        return OBD2_FRAME_READY;
    }
    if (ecu->vin_transfer_state == VIN_CF2_PENDING) {
        const uint8_t cf2[8] = {0x22u, '0', '0', '0', '0', '0', '0', '1'};
        for (uint8_t i = 0; i < 8u; ++i) response->data[i] = cf2[i];
        ecu->vin_transfer_state = 0u;
        return OBD2_FRAME_READY;
    }
    return OBD2_NO_FRAME;
}

static int process_flow_control(obd2_ecu_t *ecu, const obd2_frame_t *request,
                                uint32_t now_ms, obd2_frame_t *response)
{
    if (ecu->vin_transfer_state != VIN_WAIT_FC &&
        ecu->vin_transfer_state != VIN_WAIT_NEXT_FC)
        return OBD2_NO_FRAME;
    if (request->dlc != 8u || (request->data[0] & 0xF0u) != 0x30u) {
        ecu->vin_transfer_state = 0u;
        return OBD2_MALFORMED;
    }
    uint8_t flow_status = request->data[0] & 0x0Fu;
    if (flow_status == 1u) return OBD2_NO_FRAME; /* WAIT; overall timeout remains armed */
    if (flow_status != 0u) {
        ecu->vin_transfer_state = 0u; /* OVFLW and reserved flow status */
        return OBD2_MALFORMED;
    }
    uint8_t encoded_stmin = request->data[2];
    if (encoded_stmin <= 0x7Fu) {
        ecu->vin_stmin_ms = encoded_stmin;
    } else if (encoded_stmin >= 0xF1u && encoded_stmin <= 0xF9u) {
        ecu->vin_stmin_ms = 1u; /* 100..900us rounded up to the 1kHz tick */
    } else {
        ecu->vin_transfer_state = 0u;
        return OBD2_MALFORMED;
    }
    ecu->vin_block_remaining = request->data[1];
    ecu->vin_transfer_state = (ecu->vin_transfer_state == VIN_WAIT_FC) ?
                                  VIN_CF1_PENDING : VIN_CF2_PENDING;
    ecu->vin_next_tx_ms = now_ms + ecu->vin_stmin_ms;
    if (ecu->vin_stmin_ms == 0u) return emit_vin_cf(ecu, now_ms, response);
    return OBD2_NO_FRAME;
}

int obd2_process(obd2_ecu_t *ecu, const obd2_frame_t *request,
                 uint32_t now_ms, obd2_frame_t *response)
{
    if (request->id == FLOW_CONTROL_ID) {
        if (ecu->vin_transfer_state == 0u) return OBD2_NO_FRAME;
        if (obd2_vin_expired(now_ms, ecu->vin_started_ms)) {
            ecu->vin_transfer_state = 0u;
            return OBD2_MALFORMED;
        }
        return process_flow_control(ecu, request, now_ms, response);
    }
    if (request->id != REQUEST_ID) return OBD2_NO_FRAME;
    if (request->dlc != 8u || request->data[0] == 0u ||
        request->data[0] > 7u || request->data[0] >= request->dlc)
        return OBD2_MALFORMED;

    switch (request->data[1]) {
    case 0x01u: return mode01(request, response);
    case 0x03u: return mode03(ecu, request, response);
    case 0x04u: return mode04(ecu, request, response);
    case 0x09u: return mode09(ecu, request, now_ms, response);
    default: return negative(response, request->data[1], 0x11u);
    }
}

int obd2_poll(obd2_ecu_t *ecu, uint32_t now_ms, obd2_frame_t *response)
{
    if (ecu->vin_transfer_state != 0u &&
        obd2_vin_expired(now_ms, ecu->vin_started_ms)) {
        ecu->vin_transfer_state = 0u;
        return OBD2_NO_FRAME;
    }
    if ((ecu->vin_transfer_state == VIN_CF1_PENDING ||
         ecu->vin_transfer_state == VIN_CF2_PENDING) &&
        time_reached(now_ms, ecu->vin_next_tx_ms))
        return emit_vin_cf(ecu, now_ms, response);
    return OBD2_NO_FRAME;
}
