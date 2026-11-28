/*
 * MSP430 smoke for the imported MSP430 descriptors: one source for the x1xx
 * USART parts (MSP430F149, MSP430F1611), the x2xx USCI parts (MSP430F2617,
 * MSP430F2619) and the x5xx part (MSP430F5437). Built by
 * scripts/build_msp430_uart_smoke.sh with TI msp430-gcc 9.3.1.11 and the TI
 * device headers.
 *
 * One letter per check on UART0 (P = pass, F = fail):
 *   1  hardware multiplier through compiler-generated code (16x16 -> 32)
 *   2  hardware multiplier registers: MPYS sign extension, MAC carry
 *   3  32-bit division and remainder (libgcc)
 *   4  OFIFG is set after power-on and firmware can clear it. x5xx: no
 *      crystal is fitted, so OFIFG and XT1LFOFFG come back until the FLL
 *      reference and ACLK are moved from XT1 to REFO
 *   5  UART loopback, polled: receive flag sets, RXBUF read clears it
 *      (loopback bytes 5, 6 and 7 also leave on the transmit line: Z, k, z)
 *   6  UART receive interrupt in loopback
 *   7  LPM0: the receive interrupt wakes the CPU
 *   8  a byte from the host arrives through the receive interrupt
 *   9  x2xx and x5xx: a call into flash above 64 KiB and a read of data there
 *  10  x5xx only: the PMM core voltage step to level 1 (SLAU208 2.2.4)
 *  11  x5xx only: MPY32 with 32-bit operands, through the compiler (64-bit
 *      product) and through the registers (signed, MAC carry)
 * Then one line on the second UART, and a WDTCTL write without the password:
 * the PUC restarts the firmware, which finds WDTIFG set and reports it.
 */
#include <msp430.h>
#include <stdint.h>

#ifndef PART
#error "build with -DPART=\"<part name>\""
#endif

#if !defined(__MSP430_HAS_UCS__)
#define SFR_IFG IFG1
#endif

#if defined(__MSP430_HAS_UCS__)
/* x5xx: the SFRs are SFRIE1/SFRIFG1 and each USCI has its own UCAxIE,
 * UCAxIFG and UCAxIV. */
#define SFR_IFG SFRIFG1
#define TX_READY() (UCA0IFG & UCTXIFG)
#define RX_READY() (UCA0IFG & UCRXIFG)
#define TXBUF UCA0TXBUF
#define RXBUF UCA0RXBUF
#define RX_VECTOR USCI_A0_VECTOR
#define TX1_READY() (UCA1IFG & UCTXIFG)
#define TX1BUF UCA1TXBUF

static void uart_init(void)
{
    UCA0CTL1 = UCSWRST | UCSSEL_2;
    UCA0CTL0 = 0;
    UCA0BR0 = 8;
    UCA0BR1 = 0;
    UCA0MCTL = 0;
    UCA0CTL1 &= ~UCSWRST;

    UCA1CTL1 = UCSWRST | UCSSEL_2;
    UCA1BR0 = 8;
    UCA1BR1 = 0;
    UCA1CTL1 &= ~UCSWRST;
}

static void loopback(int on)
{
    if (on)
        UCA0STAT |= UCLISTEN;
    else
        UCA0STAT &= ~UCLISTEN;
}

static void rx_interrupt(int on)
{
    if (on)
        UCA0IE |= UCRXIE;
    else
        UCA0IE &= ~UCRXIE;
}

#define OSC_FAULTS (XT2OFFG | XT1LFOFFG | DCOFFG)

static void clear_osc_faults(void)
{
    UCSCTL7 &= ~OSC_FAULTS;
    SFRIFG1 &= ~OFIFG;
}

/* The oscillator fault of a board with no crystal: it lasts while XT1 is the
 * FLL reference or the ACLK source. */
static int osc_fault_check(void)
{
    int at_reset = (SFRIFG1 & OFIFG) && (UCSCTL7 & XT1LFOFFG);
    clear_osc_faults();
    int held = (SFRIFG1 & OFIFG) && (UCSCTL7 & XT1LFOFFG);
    UCSCTL3 = SELREF_2;
    UCSCTL4 = (UCSCTL4 & ~SELA_7) | SELA_2;
    unsigned tries = 0;
    do {
        clear_osc_faults();
    } while ((SFRIFG1 & OFIFG) && ++tries < 100);
    return at_reset && held && !(SFRIFG1 & OFIFG) && !(UCSCTL7 & OSC_FAULTS);
}

/* SetVCoreUp from the family user's guide, one level. */
static int vcore_up(unsigned level)
{
    unsigned guard = 0;
    PMMCTL0_H = PMMPW_H;
    SVSMHCTL = SVSHE | SVSHRVL0 * level | SVMHE | SVSMHRRL0 * level;
    SVSMLCTL = SVSLE | SVMLE | SVSMLRRL0 * level;
    while (!(PMMIFG & SVSMLDLYIFG) && ++guard < 1000) {
    }
    int below = (PMMIFG & SVMLIFG) != 0;
    PMMIFG &= ~(SVMLVLRIFG | SVMLIFG);
    PMMCTL0_L = PMMCOREV0 * level;
    if (PMMIFG & SVMLIFG)
        while (!(PMMIFG & SVMLVLRIFG) && ++guard < 1000) {
        }
    int reached = (PMMIFG & SVMLVLRIFG) != 0;
    SVSMLCTL = SVSLE | SVSLRVL0 * level | SVMLE | SVSMLRRL0 * level;
    PMMCTL0_H = 0;
    /* Locked again: this store must not land. */
    PMMCTL0_L = 0;
    return guard < 1000 && below && reached && PMMCTL0 == (0x9600 | level);
}

static volatile uint32_t wa = 0x89ABCDEFUL, wb = 0x12345679UL;
static volatile uint64_t wide_product;

static int mpy32_check(void)
{
    /* The store to a volatile ends the compiler's own use of MPY32 before the
     * register stores below; without it the compiler may run its multiply
     * after them and overwrite the result registers. */
    wide_product = (uint64_t)wa * wb;
    int ok = wide_product == 0x09CA39E16BEE9FF7ULL;
    /* -100000 * 70000 = -7000000000, signed 32 x 32. */
    MPYS32L = 0x7960;
    MPYS32H = 0xFFFE;
    OP2L = 0x1170;
    OP2H = 0x0001;
    ok = ok && RES0 == 0x7A00 && RES1 == 0x5EC4 && RES2 == 0xFFFE && RES3 == 0xFFFF &&
         SUMEXT == 0xFFFF;
    /* MAC: 0xFFFFFFFFFFFFFFFF + 1 * 1 carries out of 64 bits. */
    RES0 = RES1 = RES2 = RES3 = 0xFFFF;
    MAC32L = 1;
    MAC32H = 0;
    OP2L = 1;
    OP2H = 0;
    return ok && RES0 == 0 && RES3 == 0 && SUMEXT == 1 && (MPY32CTL0 & MPYC);
}
#elif defined(__MSP430_HAS_USCI__)
#define TX_READY() (IFG2 & UCA0TXIFG)
#define RX_READY() (IFG2 & UCA0RXIFG)
#define TXBUF UCA0TXBUF
#define RXBUF UCA0RXBUF
#define RX_VECTOR USCIAB0RX_VECTOR
#define TX1_READY() (UC1IFG & UCA1TXIFG)
#define TX1BUF UCA1TXBUF

static void uart_init(void)
{
    UCA0CTL1 = UCSWRST | UCSSEL_2;
    UCA0CTL0 = 0;
    UCA0BR0 = 8;
    UCA0BR1 = 0;
    UCA0MCTL = 0;
    UCA0CTL1 &= ~UCSWRST;

    UCA1CTL1 = UCSWRST | UCSSEL_2;
    UCA1BR0 = 8;
    UCA1BR1 = 0;
    UCA1CTL1 &= ~UCSWRST;
}

static void loopback(int on)
{
    if (on)
        UCA0STAT |= UCLISTEN;
    else
        UCA0STAT &= ~UCLISTEN;
}

static void rx_interrupt(int on)
{
    if (on)
        IE2 |= UCA0RXIE;
    else
        IE2 &= ~UCA0RXIE;
}
#else
#define TX_READY() (IFG1 & UTXIFG0)
#define RX_READY() (IFG1 & URXIFG0)
#define TXBUF U0TXBUF
#define RXBUF U0RXBUF
#define RX_VECTOR USART0RX_VECTOR
#define TX1_READY() (IFG2 & UTXIFG1)
#define TX1BUF U1TXBUF

static void uart_init(void)
{
    U0CTL = SWRST | CHAR;
    U0TCTL = SSEL1;
    U0BR0 = 8;
    U0BR1 = 0;
    U0MCTL = 0;
    ME1 |= UTXE0 | URXE0;
    U0CTL &= ~SWRST;

    U1CTL = SWRST | CHAR;
    U1TCTL = SSEL1;
    U1BR0 = 8;
    U1BR1 = 0;
    U1MCTL = 0;
    ME2 |= UTXE1;
    U1CTL &= ~SWRST;
}

static void loopback(int on)
{
    if (on)
        U0CTL |= LISTEN;
    else
        U0CTL &= ~LISTEN;
}

static void rx_interrupt(int on)
{
    if (on)
        IE1 |= URXIE0;
    else
        IE1 &= ~URXIE0;
}
#endif

static volatile uint8_t rx_count;
static volatile uint8_t rx_last;

__attribute__((interrupt(RX_VECTOR))) void rx_isr(void)
{
#if defined(__MSP430_HAS_UCS__)
    /* Reading UCA0IV clears UCRXIFG; the byte stays in UCA0RXBUF. */
    if (UCA0IV != USCI_UCRXIFG)
        return;
#endif
    rx_last = RXBUF;
    rx_count++;
    __bic_SR_register_on_exit(CPUOFF);
}

static void put(char c)
{
    while (!TX_READY()) {
    }
    TXBUF = c;
}

static void print(const char *s)
{
    while (*s)
        put(*s++);
}

static void print1(const char *s)
{
    while (*s) {
        while (!TX1_READY()) {
        }
        TX1BUF = *s++;
    }
}

static void hex8(uint8_t v)
{
    static const char digits[] = "0123456789ABCDEF";
    put(digits[v >> 4]);
    put(digits[v & 15]);
}

static void report(int ok)
{
    put(ok ? 'P' : 'F');
}

static volatile uint16_t ma = 1234, mb = 5678;
static volatile uint32_t big = 1000000007UL;

#if defined(__MSP430X_LARGE__)
__attribute__((section(".upper.rodata"))) static const uint16_t far_table[4] = {
    0x1357, 0x2468, 0x369C, 0x48D0,
};

__attribute__((noinline, section(".upper.text"))) static uint16_t far_sum(uint16_t seed)
{
    volatile const uint16_t *t = far_table;
    return seed + t[0] + t[1] + t[2] + t[3];
}
#endif

int main(void)
{
    uint8_t puc = SFR_IFG & WDTIFG;

    WDTCTL = WDTPW | WDTHOLD;
    uart_init();

    if (puc) {
        SFR_IFG &= ~WDTIFG;
        print(PART " PUC WDTIFG=1\r\n" PART " smoke complete\r\n");
        for (;;) {
        }
    }

    print(PART " boot\r\n" PART " checks ");

    /* 1 */
    report((uint32_t)ma * mb == 7006652UL);

    /* 2 */
    MPYS = (uint16_t)-3;
    OP2 = 7;
    int mpys = RESLO == 0xFFEB && RESHI == 0xFFFF && SUMEXT == 0xFFFF;
    RESLO = 0xFFFF;
    RESHI = 0;
    MAC = 1;
    OP2 = 1;
    report(mpys && RESLO == 0 && RESHI == 1 && SUMEXT == 0);

    /* 3 */
    report(big / 12345 == 81004UL && big % 12345 == 5627UL);

    /* 4 */
#if defined(__MSP430_HAS_UCS__)
    report(osc_fault_check());
#else
    int ofifg = (IFG1 & OFIFG) != 0;
    IFG1 &= ~OFIFG;
    report(ofifg && !(IFG1 & OFIFG));
#endif

    /* 5: loopback stays off while a result letter is sent */
    loopback(1);
    TXBUF = 0x5A;
    while (!RX_READY()) {
    }
    uint8_t got = RXBUF;
    int polled = got == 0x5A && !RX_READY();
    loopback(0);
    report(polled);

    /* 6 */
    rx_count = 0;
    rx_interrupt(1);
    __enable_interrupt();
    loopback(1);
    TXBUF = 'k';
    while (rx_count == 0) {
    }
    loopback(0);
    report(rx_count == 1 && rx_last == 'k' && !RX_READY());

    /* 7: interrupts stay off until the same instruction that stops the CPU */
    __disable_interrupt();
    rx_count = 0;
    loopback(1);
    TXBUF = 'z';
    __bis_SR_register(CPUOFF | GIE);
    loopback(0);
    report(rx_count == 1 && rx_last == 'z');

    /* 8 */
    rx_count = 0;
    while (rx_count == 0) {
    }
    report(rx_last == 'R');
    rx_interrupt(0);
    __disable_interrupt();

#if defined(__MSP430X_LARGE__)
    /* 9 */
    report(far_sum(1) == 0xB72Cu && /* 1 + the four table words */
           (uint32_t)(__int20)(void *)far_sum >= 0x10000UL);
#endif

#if defined(__MSP430_HAS_UCS__)
    /* 10 */
    report(vcore_up(1));
    /* 11 */
    report(mpy32_check());
#endif

    print(" DONE rx=");
    hex8(rx_last);
    print("\r\n");
    print1(PART " UART1 ok\r\n");

    /* A WDTCTL write without the password is a security key violation. */
    WDTCTL = WDTHOLD;
    print(PART " PUC missing\r\n");
    for (;;) {
    }
}
