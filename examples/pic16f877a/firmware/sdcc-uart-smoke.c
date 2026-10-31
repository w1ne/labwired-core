/* PIC16F877A smoke: USART TX banner + LED latch on RB0.
 *
 * Fosc = 16 MHz HS crystal, 9600 baud (BRGH = 1, SPBRG = 25; DS39582B
 * Table 10-4). Built with SDCC 4.4.0 (pic14 port) + gputils 1.4.0 and the
 * stock 16f877a_g.lkr; see build.sh.
 */
#include <pic16regs.h>

/* HS oscillator, WDT off, power-up timer on, LVP off (RB3 is digital). */
static __code unsigned int __at (_CONFIG) config = _FOSC_HS & _WDTE_OFF & _PWRTE_ON & _LVP_OFF;

static void uart_putc(char c)
{
    while (!TRMT)
        ;
    TXREG = c;
}

static void uart_puts(const char *s)
{
    while (*s)
        uart_putc(*s++);
}

void main(void)
{
    unsigned char i;

    TRISC6 = 0;
    SPBRG = 25;
    TXSTA = 0x24;             /* TXEN | BRGH */
    RCSTA = 0x80;             /* SPEN */

    TRISB0 = 0;
    RB0 = 1;

    uart_puts("PIC16F877A OK\r\n");
    uart_puts(RB0 ? "LED=1\r\n" : "LED=0\r\n");

    for (i = 0; i < 3; i++)
        RB0 ^= 1;
    uart_puts(RB0 ? "LED=0?\r\n" : "TOGGLE OK\r\n");

    for (;;)
        ;
}
