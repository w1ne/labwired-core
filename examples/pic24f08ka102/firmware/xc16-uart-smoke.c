/*
 * PIC24F08KA102 UART1 smoke for the public LabWired PIC24 model.
 *
 * Built with Microchip XC16 v1.70 and the stock p24F08KA102.gld linker
 * script. The program configures UART1, transmits "HI", then toggles
 * RA0/LATA0 forever. RA0 is bonded on the 28-pin package.
 */

#include <xc.h>

static void uart_putc(char c)
{
    while (U1STAbits.UTXBF) {
        /* wait for the transmit buffer */
    }
    U1TXREG = c;
}

int main(void)
{
    U1MODE = 0x0000;
    U1STA = 0x0000;
    U1BRG = 0x0000;
    U1MODEbits.UARTEN = 1;
    U1STAbits.UTXEN = 1;

    uart_putc('H');
    uart_putc('I');

    TRISA &= ~0x0001;
    for (;;) {
        LATA ^= 0x0001;
    }
}
