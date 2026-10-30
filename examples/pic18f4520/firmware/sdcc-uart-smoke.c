/* PIC18F4520 L1 smoke: EUSART TX banner + LED latch on RB0. */
#include <pic18fregs.h>

#pragma config OSC = HS, WDT = OFF, LVP = OFF, PBADEN = OFF, XINST = OFF

static void uart_putc(char c)
{
    while (!TXSTAbits.TRMT)
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

    TRISCbits.TRISC6 = 0;
    SPBRG = 25;               /* 9600 baud at Fosc = 16 MHz, BRGH = 1 */
    TXSTA = 0x24;             /* TXEN | BRGH */
    RCSTA = 0x80;             /* SPEN */

    TRISBbits.TRISB0 = 0;
    LATBbits.LATB0 = 1;

    uart_puts("PIC18F4520 OK\r\n");
    uart_puts(LATBbits.LATB0 ? "LED=1\r\n" : "LED=0\r\n");

    for (i = 0; i < 3; i++)
        LATBbits.LATB0 ^= 1;
    uart_puts(LATBbits.LATB0 ? "LED=0?\r\n" : "TOGGLE OK\r\n");

    for (;;)
        ;
}
