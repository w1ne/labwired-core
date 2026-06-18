/*
 * Singleton bridge around the real `iolinki` device stack
 * (core/third_party/iolinki). The device stack keeps all state in globals
 * (`iolink_init`/`iolink_process` take no context), so only ONE device
 * instance may be alive at a time. `g_device_in_use` enforces that; multi-port
 * stack-backed devices require the reentrancy work tracked in
 * docs/engineering/iolink-device-stack-isolation.md.
 *
 * GPL: this links the GPL-3.0-or-later `iolinki` device stack and is only
 * compiled under the (non-default) `iolink-native` feature.
 */
#include "iolink_device_bridge.h"

#include "iolinki/application.h"
#include "iolinki/iolink.h"
#include "iolinki/phy.h"

#include <string.h>

#define LW_IOLD_QUEUE_CAP 256U

typedef struct {
    /* `iolink_init` retains the PHY by pointer, so it must outlive init. */
    iolink_phy_api_t phy;
    uint8_t tx_queue[LW_IOLD_QUEUE_CAP]; /* device -> master */
    size_t tx_head;
    size_t tx_len;
    uint8_t rx_queue[LW_IOLD_QUEUE_CAP]; /* master -> device */
    size_t rx_head;
    size_t rx_len;
} lw_iold_context_t;

/*
 * Routes the device PHY callbacks. Thread-local so it never races with master
 * bridge calls on other threads. `g_device_in_use` stays process-global: the
 * device stack keeps its state in a true global (`g_dll_ctx`), so only one
 * device may exist process-wide regardless of thread.
 */
static __thread lw_iold_context_t* g_device_active;
static int g_device_in_use;

static void q_push(uint8_t* q, size_t* head, size_t* len, uint8_t byte)
{
    if (*len >= LW_IOLD_QUEUE_CAP) {
        return;
    }
    size_t tail = (*head + *len) % LW_IOLD_QUEUE_CAP;
    q[tail] = byte;
    *len += 1U;
}

static int q_pop(uint8_t* q, size_t* head, size_t* len, uint8_t* byte)
{
    if (*len == 0U) {
        return 0;
    }
    *byte = q[*head];
    *head = (*head + 1U) % LW_IOLD_QUEUE_CAP;
    *len -= 1U;
    return 1;
}

static int dev_send(const uint8_t* data, size_t len)
{
    if ((g_device_active == 0) || (data == 0)) {
        return -1;
    }
    for (size_t i = 0; i < len; i++) {
        q_push(g_device_active->tx_queue, &g_device_active->tx_head,
               &g_device_active->tx_len, data[i]);
    }
    return (int) len;
}

static int dev_recv_byte(uint8_t* byte)
{
    if ((g_device_active == 0) || (byte == 0)) {
        return -1;
    }
    return q_pop(g_device_active->rx_queue, &g_device_active->rx_head,
                 &g_device_active->rx_len, byte);
}

size_t lw_iold_context_size(void)
{
    return sizeof(lw_iold_context_t);
}

int lw_iold_init_proximity(void* ctx, int present)
{
    if (ctx == 0) {
        return -1;
    }
    if (g_device_in_use) {
        return -2;
    }

    lw_iold_context_t* c = (lw_iold_context_t*) ctx;
    memset(c, 0, sizeof(*c));
    c->phy.send = dev_send;
    c->phy.recv_byte = dev_recv_byte;

    iolink_config_t cfg = {
        .m_seq_type = IOLINK_M_SEQ_TYPE_2_1,
        .min_cycle_time = 20U,
        .pd_in_len = 1U,
        .pd_out_len = 0U,
        .t_pd_us = 0U,
    };

    g_device_active = c;
    int ret = iolink_init(&c->phy, &cfg);
    if (ret != 0) {
        g_device_active = 0;
        return ret;
    }
    iolink_set_timing_enforcement(false);

    uint8_t pd = present ? 0x01U : 0x00U;
    (void) iolink_pd_input_update(&pd, 1U, true);

    g_device_in_use = 1;
    return 0;
}

size_t lw_iold_feed_master(void* ctx, const uint8_t* data, size_t len)
{
    if ((ctx == 0) || (data == 0)) {
        return 0U;
    }
    lw_iold_context_t* c = (lw_iold_context_t*) ctx;

    /*
     * The master's wake-up is a C/Q current pulse on real hardware, not a UART
     * frame. Our master bridge models it as a single 0x55 byte; forwarding it
     * into the device's frame assembler would offset the first startup frame
     * by one byte. Swallow it at the wire boundary, like the reference
     * master_loopback_demo PHY does.
     */
    if ((len == 1U) && (data[0] == 0x55U)) {
        return len;
    }

    for (size_t i = 0; i < len; i++) {
        q_push(c->rx_queue, &c->rx_head, &c->rx_len, data[i]);
    }

    g_device_active = c;
    iolink_process();
    return len;
}

size_t lw_iold_drain_tx(void* ctx, uint8_t* out, size_t out_len)
{
    if ((ctx == 0) || (out == 0)) {
        return 0U;
    }
    lw_iold_context_t* c = (lw_iold_context_t*) ctx;
    g_device_active = c;
    size_t n = 0U;
    while (n < out_len && q_pop(c->tx_queue, &c->tx_head, &c->tx_len, &out[n]) > 0) {
        n++;
    }
    return n;
}
