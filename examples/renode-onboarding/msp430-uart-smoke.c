/*
 * MSP430 smoke for the imported MSP430 descriptors: one source for the x1xx
 * USART parts (MSP430F149, MSP430F1611) and the x2xx USCI parts (MSP430F2617,
 * MSP430F2619). Built by scripts/build_msp430_uart_smoke.sh with TI msp430-gcc
 * 9.3.1.11 and the TI device headers.
 *
 * One letter per check on UART0 (P = pass, F = fail):
 *   1  hardware multiplier through compiler-generated code (16x16 -> 32)
 *   2  hardware multiplier registers: MPYS sign extension, MAC carry
 *   3  32-bit division and remainder (libgcc)
 *   4  OFIFG is set after power-on and firmware can clear it
 *   5  UART loopback, polled: receive flag sets, RXBUF read clears it
 *      (loopback bytes 5, 6 and 7 also leave on the transmit line: Z, k, z)
 *   6  UART receive interrupt in loopback
 *   7  LPM0: the receive interrupt wakes the CPU
 *   8  a byte from the host arrives through the receive interrupt
 *   9  x2xx only: a call into flash above 64 KiB and a read of data there
 * Then one line on the second UART, and a WDTCTL write without the password:
 * the PUC restarts the firmware, which finds WDTIFG set and reports it.
 */
#include <msp430.h>
#include <stdint.h>

#ifndef PART
#error "build with -DPART=\"<part name>\""
#endif

#if defined(__MSP430_HAS_USCI__)
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
    uint8_t puc = IFG1 & WDTIFG;

    WDTCTL = WDTPW | WDTHOLD;
    uart_init();

    if (puc) {
        IFG1 &= ~WDTIFG;
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
    int ofifg = (IFG1 & OFIFG) != 0;
    IFG1 &= ~OFIFG;
    report(ofifg && !(IFG1 & OFIFG));

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
