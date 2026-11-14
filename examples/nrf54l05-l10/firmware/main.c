/* nRF54L05/L10 real-register Cortex-M33 smoke; UARTE20 EasyDMA, GPIO P2. */
#include <stdint.h>
#include "../../nrf54l15-dk/src/nrf54l15.h"
#if defined(NRF54L05)
#define CHIP_ID "NRF54L05_OK\r\n"
#define MEM_GEOM "rram=500K ram=96K\r\n"
#elif defined(NRF54L10)
#define CHIP_ID "NRF54L10_OK\r\n"
#define MEM_GEOM "rram=1012K ram=192K\r\n"
#else
#error "Build must select exact nRF54L sibling"
#endif
/* EasyDMA requires the source buffer in SRAM, not RRAM. */
static char tx_buf[100];
static void tx(const char *s) {
    uint32_t n=0;
    while (s[n]) { tx_buf[n]=s[n]; n++; }
    UARTE_EVENTS_DMA_TX_END(UARTE20_BASE)=0;
    UARTE_EVENTS_TXSTOPPED(UARTE20_BASE)=0;
    UARTE_DMA_TX_PTR(UARTE20_BASE)=(uint32_t)(uintptr_t)tx_buf;
    UARTE_DMA_TX_MAXCNT(UARTE20_BASE)=n;
    UARTE_TASKS_DMA_TX_START(UARTE20_BASE)=1;
    while (UARTE_EVENTS_DMA_TX_END(UARTE20_BASE)==0) {}
    UARTE_EVENTS_DMA_TX_END(UARTE20_BASE)=0;
    UARTE_TASKS_DMA_TX_STOP(UARTE20_BASE)=1;
}
static void spin(uint32_t n) { while (n--) __asm__ volatile ("nop"); }
int main(void) {
    GPIO_DIRSET(GPIO_P2_BASE)=1u<<LED0_PIN;
    UARTE_PSEL_TXD(UARTE20_BASE)=UARTE20_PIN_TXD;
    UARTE_PSEL_RXD(UARTE20_BASE)=UARTE20_PIN_RXD;
    UARTE_BAUDRATE(UARTE20_BASE)=UARTE_BAUD_115200;
    UARTE_ENABLE(UARTE20_BASE)=UARTE_ENABLE_UARTE;
    tx(CHIP_ID);
    tx(MEM_GEOM);
    for (;;) {
        GPIO_OUTSET(GPIO_P2_BASE)=1u<<LED0_PIN;
        spin(350);
        GPIO_OUTCLR(GPIO_P2_BASE)=1u<<LED0_PIN;
        spin(350);
    }
}
