/* nRF52840 registers the bus-over-nets firmware uses (nRF52840 PS v1.11).
 * Instance 0 of the shared serial window at 0x40003000: ENABLE picks SPIM
 * (7), SPIS (2), TWIM (6) or TWIS (9). */
#ifndef NRF52_H
#define NRF52_H
#include <stdint.h>

#define REG(a) (*(volatile uint32_t *)(a))

/* GPIO P0 */
#define P0_OUTSET     REG(0x50000508u)
#define P0_OUTCLR     REG(0x5000050Cu)
#define P0_DIRSET     REG(0x50000518u)
#define P0_PIN_CNF(n) REG(0x50000700u + 4u * (n))

/* Serial instance 0 (SPIM0 / SPIS0 / TWIM0 / TWIS0) */
#define S0(off) REG(0x40003000u + (off))
#define S0_ENABLE   S0(0x500)
#define S0_SHORTS   S0(0x200)
#define S0_INTENSET S0(0x304)
#define S0_RXD_PTR    S0(0x534)
#define S0_RXD_MAXCNT S0(0x538)
#define S0_RXD_AMOUNT S0(0x53C)
#define S0_TXD_PTR    S0(0x544)
#define S0_TXD_MAXCNT S0(0x548)
#define S0_TXD_AMOUNT S0(0x54C)
#define S0_ORC      S0(0x5C0)

/* SPIM */
#define SPIM_TASKS_START   S0(0x010)
#define SPIM_EVENTS_ENDRX  S0(0x110)
#define SPIM_EVENTS_END    S0(0x118)
#define SPIM_EVENTS_ENDTX  S0(0x120)
#define SPIM_EVENTS_STARTED S0(0x14C)
#define SPIM_PSEL_SCK  S0(0x508)
#define SPIM_PSEL_MOSI S0(0x50C)
#define SPIM_PSEL_MISO S0(0x510)
#define SPIM_FREQUENCY S0(0x524)
#define SPIM_CONFIG    S0(0x554)

/* SPIS */
#define SPIS_TASKS_ACQUIRE  S0(0x024)
#define SPIS_TASKS_RELEASE  S0(0x028)
#define SPIS_EVENTS_END     S0(0x104)
#define SPIS_EVENTS_ENDRX   S0(0x110)
#define SPIS_EVENTS_ACQUIRED S0(0x128)
#define SPIS_SEMSTAT  S0(0x400)
#define SPIS_STATUS   S0(0x440)
#define SPIS_PSEL_SCK  S0(0x508)
#define SPIS_PSEL_MISO S0(0x50C)
#define SPIS_PSEL_MOSI S0(0x510)
#define SPIS_PSEL_CSN  S0(0x514)
#define SPIS_CONFIG    S0(0x554)
#define SPIS_DEF       S0(0x55C)

/* TWIM */
#define TWIM_TASKS_STARTRX S0(0x000)
#define TWIM_TASKS_STARTTX S0(0x008)
#define TWIM_TASKS_STOP    S0(0x014)
#define TWIM_TASKS_RESUME  S0(0x020)
#define TWIM_EVENTS_STOPPED S0(0x104)
#define TWIM_EVENTS_ERROR   S0(0x124)
#define TWIM_EVENTS_LASTRX  S0(0x15C)
#define TWIM_EVENTS_LASTTX  S0(0x160)
#define TWIM_ERRORSRC  S0(0x4C4)
#define TWIM_PSEL_SCL  S0(0x508)
#define TWIM_PSEL_SDA  S0(0x50C)
#define TWIM_FREQUENCY S0(0x524)
#define TWIM_ADDRESS   S0(0x588)
#define TWIM_SHORT_LASTTX_STARTRX (1u << 7)
#define TWIM_SHORT_LASTTX_STOP    (1u << 9)
#define TWIM_SHORT_LASTRX_STOP    (1u << 12)

/* TWIS */
#define TWIS_TASKS_PREPARERX S0(0x030)
#define TWIS_TASKS_PREPARETX S0(0x034)
#define TWIS_EVENTS_STOPPED  S0(0x104)
#define TWIS_EVENTS_ERROR    S0(0x124)
#define TWIS_EVENTS_WRITE    S0(0x164)
#define TWIS_EVENTS_READ     S0(0x168)
#define TWIS_PSEL_SCL  S0(0x508)
#define TWIS_PSEL_SDA  S0(0x50C)
#define TWIS_ADDRESS0  S0(0x588)
#define TWIS_CONFIG    S0(0x594)

/* SPIM3 (0x4002F000): its own window, with the hardware chip select and
 * D/CX registers SPIM0..2 lack. */
#define SPIM3(off) REG(0x4002F000u + (off))
#define SPIM3_TASKS_START     SPIM3(0x010)
#define SPIM3_EVENTS_END      SPIM3(0x118)
#define SPIM3_INTENSET        SPIM3(0x304)
#define SPIM3_ENABLE          SPIM3(0x500)
#define SPIM3_PSEL_SCK        SPIM3(0x508)
#define SPIM3_PSEL_MOSI       SPIM3(0x50C)
#define SPIM3_PSEL_MISO       SPIM3(0x510)
#define SPIM3_PSEL_CSN        SPIM3(0x514)
#define SPIM3_FREQUENCY       SPIM3(0x524)
#define SPIM3_RXD_PTR         SPIM3(0x534)
#define SPIM3_RXD_MAXCNT      SPIM3(0x538)
#define SPIM3_RXD_AMOUNT      SPIM3(0x53C)
#define SPIM3_TXD_PTR         SPIM3(0x544)
#define SPIM3_TXD_MAXCNT      SPIM3(0x548)
#define SPIM3_TXD_AMOUNT      SPIM3(0x54C)
#define SPIM3_CONFIG          SPIM3(0x554)
#define SPIM3_IFTIMING_CSNDUR SPIM3(0x564)
#define SPIM3_CSNPOL          SPIM3(0x568)
#define SPIM3_PSELDCX         SPIM3(0x56C)
#define SPIM3_DCXCNT          SPIM3(0x570)
#define SPIM3_ORC             SPIM3(0x5C0)

#define NVIC_ISER REG(0xE000E100u)
#define NVIC_ISER1 REG(0xE000E104u)
#define IRQ_SERIAL0 3u
#define IRQ_SPIM3 47u

/* Results the tests read, and the EasyDMA buffers, by address. */
#define RESULT ((volatile uint32_t *)0x20000100u)
#define TXBUF  ((volatile uint8_t *)0x20000200u)
#define RXBUF  ((volatile uint8_t *)0x20000300u)

/* GPIO PIN_CNF: input, input buffer connected, no pull. */
#define PIN_INPUT 0u
/* GPIO PIN_CNF: output, input buffer connected. */
#define PIN_OUTPUT 1u
/* GPIO PIN_CNF: input with S0D1 drive (open drain), as TWI pins are set up. */
#define PIN_TWI ((6u << 8) | 0u)

static inline void delay(uint32_t loops) {
    for (volatile uint32_t i = 0; i < loops; ++i) {}
}

void reset(void);
void default_handler(void) { for (;;) {} }
#endif
