#ifndef IOLINKI_MASTER_TESTS_FAKE_IOLINK_DEVICE_H
#define IOLINKI_MASTER_TESTS_FAKE_IOLINK_DEVICE_H

#include <stdbool.h>
#include <stdint.h>

#include "iolinki/phy.h"

void fake_iolink_device_reset(uint8_t pd_in_value, uint8_t pd_in_len, uint8_t od_len);
void fake_iolink_device_set_isdu_object(uint16_t index, uint8_t subindex, const uint8_t* data, uint8_t len);
void fake_iolink_device_set_direct_parameter_page1(uint8_t min_cycle_time,
                                                   uint8_t mseq_capability,
                                                   uint8_t pd_in_descriptor,
                                                   uint8_t pd_out_descriptor,
                                                   uint16_t vendor_id,
                                                   uint32_t device_id);
void fake_iolink_device_set_data_storage(const uint8_t* data, uint8_t len);
void fake_iolink_device_set_event_pending(bool pending);
void fake_iolink_device_set_event_code(uint16_t event_code);
void fake_iolink_device_set_event_memory(const uint8_t* memory, uint8_t len);
void fake_iolink_device_corrupt_next_response_checksum(void);
void fake_iolink_device_drop_next_response(void);
void fake_iolink_device_truncate_next_response(void);
/** @brief Keep already queued but unread reply bytes when the next reply is queued.
 *
 * Models a reply that is already on the wire when the master sends its next
 * request: the unread bytes stay in front of the new reply.
 */
void fake_iolink_device_keep_unread_responses(void);
/** @brief Drop all queued (read and unread) reply bytes, like a PHY RX flush. */
void fake_iolink_device_flush_rx(void);
const iolink_phy_api_t* fake_iolink_device_phy(void);
uint32_t fake_iolink_device_wakeup_count(void);
uint32_t fake_iolink_device_transition_count(void);
uint32_t fake_iolink_device_operate_cycle_count(void);

#endif /* IOLINKI_MASTER_TESTS_FAKE_IOLINK_DEVICE_H */
