#ifndef OBD2_H
#define OBD2_H

#include <stdbool.h>
#include <stdint.h>

enum { OBD2_NO_FRAME = 0, OBD2_FRAME_READY = 1, OBD2_MALFORMED = -1 };
enum { OBD2_TX_PENDING = 0, OBD2_TX_OK = 1, OBD2_TX_FAILED = -1 };

typedef struct {
    uint32_t id;
    uint8_t dlc;
    uint8_t data[8];
} obd2_frame_t;

typedef struct {
    uint8_t dtc_count;
    uint8_t vin_transfer_state;
    uint8_t vin_block_remaining;
    uint8_t vin_stmin_ms;
    uint32_t vin_started_ms;
    uint32_t vin_next_tx_ms;
} obd2_ecu_t;

void obd2_init(obd2_ecu_t *ecu);
int obd2_process(obd2_ecu_t *ecu, const obd2_frame_t *request,
                 uint32_t now_ms, obd2_frame_t *response);
int obd2_poll(obd2_ecu_t *ecu, uint32_t now_ms, obd2_frame_t *response);
bool obd2_vin_expired(uint32_t now_ms, uint32_t started_ms);
int obd2_tx_status(uint32_t tsr);

#endif
