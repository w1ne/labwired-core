/*
 * Real XC16-built firmware for the dsPIC33FJ32MC204 model.
 *
 * Build (XC16 v1.70 in the fstlx/xc16 container):
 *   xc16-gcc -mcpu=33FJ32MC204 -O0 -Wall -o dspic33fj32mc204-xc16-uart-smoke.elf \
 *       xc16-uart-smoke.c
 *
 * The program configures UART1, transmits "HI", then toggles RB0/LATB0
 * forever. It deliberately goes through a helper function so the image
 * exercises the compiler's real call/return sequence and the linker's
 * default startup code, not a hand-written instruction fixture.
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
    U1BRG = 0x0000; /* the model does not derive baud timing from the clock */
    U1MODEbits.UARTEN = 1;
    U1STAbits.UTXEN = 1;

    uart_putc('H');
    uart_putc('I');

    TRISB &= ~0x0001;
    for (;;) {
        LATB ^= 0x0001;
    }
}
