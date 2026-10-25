/*
 * Real XC16 firmware for the dsPIC33FJ32MC202 M1 package/GPIO milestone.
 *
 * Build (XC16 v1.70 container):
 *   xc16-gcc -mcpu=33FJ32MC202 -O0 -Wall \
 *     -Wl,-T,/opt/microchip/xc16/support/dsPIC33F/gld/p33FJ32MC202.gld \
 *     -o dspic33fj32mc202-xc16-m1-gpio.elf xc16-m1-gpio.c
 *
 * The 28-pin 202 exposes RA0-RA4 and RB0-RB15 (no PORTC). The program
 * prints "G1" on UART1, makes PORTA/PORTB outputs and toggles RA0/RB0
 * forever. The integration test checks the package masks, the direction
 * registers and both pad levels.
 */

#include <xc.h>

static void uart_putc(char c)
{
    while (U1STAbits.UTXBF) {
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

    uart_putc('G');
    uart_putc('1');

    TRISA = 0x0000; /* RA0-RA4 outputs */
    TRISB = 0x0000; /* RB0-RB15 outputs */

    for (;;) {
        LATA ^= 0x0001;
        LATB ^= 0x0001;
    }
}
