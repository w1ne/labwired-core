#ifndef UDS_ECU_APP_H
#define UDS_ECU_APP_H

#include "uds/uds_core.h"

/* Board-provided: emit a NUL-terminated trace string via the board UART. */
void uds_ecu_app_log(const char *msg);

/* Populate the board-agnostic UDS handlers, DID table, and a seeded DTC store
 * into `cfg`. Call AFTER setting the board-specific cfg fields and BEFORE
 * uds_init. `vin` must point to a 17-byte VIN (reported by DID 0xF190). */
void uds_ecu_app_fill_config(uds_config_t *cfg, const char *vin);

#endif /* UDS_ECU_APP_H */
