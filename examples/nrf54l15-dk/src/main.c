/*
 * nRF54L15 onboarding smoke firmware.
 *
 * Goal: prove the chip profile boots and that UARTE20 EasyDMA reaches the
 * capture sink. Exercises, in order:
 *   1. reset -> vector table at RRAM 0x0 -> .data copy / .bss zero
 *   2. GPIO P2 (LED0 on P2.09) — DIRSET/OUTSET, the 11-pin port
 *   3. UARTE20 EasyDMA TX at 115200 -> banner
 *
 * Everything is polled. No interrupts, no clock setup: the part comes out of
 * reset on the internal HFOSC, which is why this boots without a CLOCK model.
 */
#include <stdint.h>

#include "nrf54l15.h"

/*
 * EasyDMA reads the TX buffer over the bus, so it must live in RAM, not RRAM.
 * Marking it non-const and writing it at runtime forces it into .data — a
 * string literal passed directly would sit in RRAM and, on real silicon,
 * EasyDMA would fault. Getting this wrong is the classic first-UARTE bug, and
 * a simulator that permits it would be modelling the part too leniently.
 */
static char tx_buf[128];

static uint32_t str_copy(char *dst, const char *src)
{
    uint32_t n = 0;
    while (src[n] != '\0') {
        dst[n] = src[n];
        n++;
    }
    return n;
}

static void uarte_init(void)
{
    UARTE_PSEL_TXD(UARTE20_BASE) = UARTE20_PIN_TXD;
    UARTE_PSEL_RXD(UARTE20_BASE) = UARTE20_PIN_RXD;
    UARTE_BAUDRATE(UARTE20_BASE) = UARTE_BAUD_115200;
    UARTE_ENABLE(UARTE20_BASE)   = UARTE_ENABLE_UARTE;
}

static void uarte_write(const char *s)
{
    uint32_t len = str_copy(tx_buf, s);

    if (len == 0) {
        return;
    }

    UARTE_EVENTS_ENDTX(UARTE20_BASE) = 0;
    UARTE_TXD_PTR(UARTE20_BASE)      = (uint32_t)(uintptr_t)tx_buf;
    UARTE_TXD_MAXCNT(UARTE20_BASE)   = len;
    UARTE_TASKS_STARTTX(UARTE20_BASE) = 1;

    while (UARTE_EVENTS_ENDTX(UARTE20_BASE) == 0) {
        /* EasyDMA completes in the model on the tick the task is written. */
    }

    UARTE_EVENTS_ENDTX(UARTE20_BASE) = 0;
    UARTE_TASKS_STOPTX(UARTE20_BASE) = 1;
}

static void led_init_and_set(void)
{
    GPIO_DIRSET(GPIO_P2_BASE) = (1u << LED0_PIN);
    GPIO_OUTSET(GPIO_P2_BASE) = (1u << LED0_PIN);
}

int main(void)
{
    led_init_and_set();
    uarte_init();

    uarte_write("nRF54L15 boot OK\r\n");
    uarte_write("core=cortex-m33 rram=1524K ram=256K\r\n");
    uarte_write("uarte20@0x500C6000 gpio2@0x50050400\r\n");

    for (;;) {
    }
}
