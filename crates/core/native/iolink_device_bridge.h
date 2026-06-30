#ifndef LABWIRED_IOLINK_DEVICE_BRIDGE_H
#define LABWIRED_IOLINK_DEVICE_BRIDGE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

size_t lw_iold_context_size(void);
/* Returns 0 on success, -1 on bad args, -2 if a device is already in use. */
int lw_iold_init_proximity(void* ctx, int present);
size_t lw_iold_feed_master(void* ctx, const uint8_t* data, size_t len);
size_t lw_iold_drain_tx(void* ctx, uint8_t* out, size_t out_len);

#ifdef __cplusplus
}
#endif

#endif
