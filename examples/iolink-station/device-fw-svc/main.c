/* IO-Link SERVICE-RICH DEVICE firmware-under-test for the simulated STM32L476.
 *
 * A companion to device (iolink-dido) that exercises the full iolinki-master
 * service surface on the wire: it advertises device_info (vendor/product/ids),
 * mirrors the master's cyclic PD-output back as PD-input, serves Data Storage
 * (ISDU 0x0003) via a RAM-backed NVM, and fires a single WARNING event when it
 * receives the sentinel PD-out byte 0xE7. Every observable is mirrored into a
 * volatile global that the Rust integration test reads by ELF symbol.
 *
 * Built as a standard STM32CubeL4 project (CMSIS startup/system/linker), with
 * peripherals driven through the CMSIS register definitions — no hand-computed
 * register addresses. There is NO SPI shifter here (unlike iolink-dido): the PD
 * this device publishes is the echo of what the master last wrote.
 */
#include "stm32l476xx.h"
#include "iolinki/application.h"
#include "iolinki/data_storage.h"
#include "iolinki/device.h"
#include "iolinki/device_info.h"
#include "iolinki/events.h"
#include "iolinki/iolink.h"
#include "iolinki/params.h"
#include "phy_labwired.h"
#include "debug_uart.h"
#include <string.h>
#include <stdint.h>

/* The CMSIS startup calls __libc_init_array to run C++/constructor init-array
 * entries; this firmware has none, and -nostartfiles drops the crt object that
 * defines _init, so provide an empty implementation (matches the other LabWired
 * Cube examples). Plain C globals are initialised by the startup .data copy. */
void __libc_init_array(void) {}

/* Observability for the host-side test harness. */
volatile uint8_t g_device_state = 0xFFu; /* iolink_dll_state_t (OPERATE==...) */
volatile uint8_t g_event_fired = 0u;     /* 1 once the 0xE7 WARNING is queued */

/* Device identity served over the mandatory ISDU indices (vendor_name 0x0010,
 * product_name 0x0012, vendor_id 0x000A, device_id 0x000B). Serving is built
 * into the stack once cfg.device_info is non-NULL. */
static const iolink_device_info_t DEVICE_INFO = {
    .vendor_name = "LABWIRED",
    .product_name = "SVCDEV",
    .vendor_id = 0x1234u,
    .device_id = 0x00056789u,
};

/* RAM-backed Data-Storage NVM. cfg.ds_storage MUST be non-NULL for the device
 * to wire its DS engine into the ISDU dispatcher (device.c: isdu.ds_ctx is only
 * set when config->ds_storage != NULL) — otherwise ISDU 0x0003 returns
 * SERVICE_NOT_AVAIL. The DS engine keeps its parameter image in its own ctx;
 * these hooks are the persist-to-NVM seam and just succeed against a byte array. */
static uint8_t g_ds_nvm[256];

static int ds_nvm_read(uint32_t addr, uint8_t *buf, size_t len) {
    if ((size_t)addr + len > sizeof(g_ds_nvm)) {
        return -1;
    }
    memcpy(buf, &g_ds_nvm[addr], len);
    return 0;
}
static int ds_nvm_write(uint32_t addr, const uint8_t *buf, size_t len) {
    if ((size_t)addr + len > sizeof(g_ds_nvm)) {
        return -1;
    }
    memcpy(&g_ds_nvm[addr], buf, len);
    return 0;
}
static int ds_nvm_erase(uint32_t addr, size_t len) {
    if ((size_t)addr + len > sizeof(g_ds_nvm)) {
        return -1;
    }
    memset(&g_ds_nvm[addr], 0xFF, len);
    return 0;
}
static const iolink_ds_storage_api_t DS_STORAGE = {
    .read = ds_nvm_read,
    .write = ds_nvm_write,
    .erase = ds_nvm_erase,
};

/* RCC (STM32L4, RM0351 §6.4) — enable peripheral clocks before touching their
 * registers. The simulator models clock-gating: USART1 (debug, APB2) and USART2
 * (IO-Link C/Q PHY, APB1) are unclocked out of reset and their register writes
 * are dropped until the matching enable bit is set. No SPI is used here. */
static void rcc_init(void) {
    RCC->APB2ENR |= RCC_APB2ENR_USART1EN;   /* debug UART */
    RCC->APB1ENR1 |= RCC_APB1ENR1_USART2EN; /* IO-Link C/Q PHY */
}

int main(void) {
    rcc_init();
    dbg_uart_init();
    dbg_puts("IOLINK SVCDEV BOOT\r\n");

    /* Zero the whole struct first: on this toolchain (arm-none-eabi GCC, -Os,
     * short-enums) a designated-initializer that leaves a field uninitialised
     * can arm a bogus timing value. memset + explicit assignment is robust
     * (see examples/iolink-dido/firmware/main.c:64). */
    iolink_device_ctx_t device;
    iolink_device_config_t cfg;
    memset(&device, 0, sizeof(device));
    memset(&cfg, 0, sizeof(cfg));
    cfg.phy = *iolink_phy_labwired_get();
    cfg.stack.m_seq_type = IOLINK_M_SEQ_TYPE_1_1; /* PD(fixed)+OD(1); carries PD-out */
    cfg.stack.min_cycle_time = 0;
    cfg.stack.pd_in_len = 1;
    cfg.stack.pd_out_len = 1; /* now consumes the master's PD-out byte */
    cfg.stack.t_pd_us = 0;
    cfg.device_info = &DEVICE_INFO;
    cfg.ds_storage = &DS_STORAGE;
    /* The ISDU handler serves the mandatory identity indices (vendor_name 0x0010,
     * etc.) from the stack's LEGACY device-info global via iolink_device_info_get()
     * (isdu.c:246) — NOT from the per-instance ctx that iolink_device_init()
     * populates from cfg.device_info. iolink_device_init() does not touch the
     * legacy global, so without this explicit registration the handler would
     * serve the built-in k_default_info ("iolinki") instead of our identity.
     * Register DEVICE_INFO into the legacy global so ISDU 0x0010 reads "LABWIRED". */
    iolink_device_info_init(&DEVICE_INFO);
    /* Initialise the parameter subsystem (the ISDU mandatory-index + tag/DS
     * handlers read through it). iolink_device_init() does NOT do this — the
     * stack's own unit tests call iolink_params_init() explicitly before any
     * ISDU read (see third_party/iolinki/tests/test_isdu.c). */
    iolink_params_init();
    if (iolink_device_init(&device, &cfg) != 0) {
        dbg_puts("IOLINK INIT FAIL\r\n");
        for (;;) {
        }
    }
    iolink_device_set_timing_enforcement(&device, false);
    dbg_puts("IOLINK INIT OK\r\n");

    iolink_dll_state_t last = (iolink_dll_state_t)0xFF;
    for (;;) {
        /* Read the master's cyclic PD-output and echo it straight back as this
         * device's PD-input (mirror loop). iolink_device_pd_output_read returns
         * the number of bytes copied (>=1 == a real PD-out byte is present, 0
         * before OPERATE, -1 on bad args) — NOT 0-for-success. */
        uint8_t out = 0u;
        if (iolink_device_pd_output_read(&device, &out, 1) >= 1) {
            (void)iolink_device_pd_input_update(&device, &out, 1, true); /* mirror */
            if (out == 0xE7u && !g_event_fired) {
                iolink_event_trigger(iolink_device_get_events_ctx(&device), 0x8CA0u,
                                     IOLINK_EVENT_TYPE_WARNING);
                g_event_fired = 1u;
                dbg_puts("EVENT FIRED\r\n");
            }
        } else {
            uint8_t idle = 0x00u;
            (void)iolink_device_pd_input_update(&device, &idle, 1, true);
        }
        iolink_device_process(&device);
        /* Deliberately do NOT advance a millisecond tick: the CPU loops far
         * faster than the simulated UART byte rate, so a per-loop tick would
         * race the stack's timeouts. With the clock frozen and timing
         * enforcement off, the handshake is driven purely by byte arrival. */

        iolink_dll_state_t s = iolink_device_get_state(&device);
        g_device_state = (uint8_t)s;
        if (s != last) {
            last = s;
            dbg_puts("STATE=");
            dbg_hex8((unsigned char)s);
            if (s == IOLINK_DLL_STATE_OPERATE) {
                dbg_puts(" OPERATE");
            }
            dbg_puts("\r\n");
        }
    }
}
