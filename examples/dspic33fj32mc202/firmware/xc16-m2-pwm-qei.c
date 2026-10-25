/*
 * Real XC16 firmware exercising the dsPIC33F MCPWM1, QEI1 and T1 timer on
 * the 28-pin dsPIC33FJ32MC202 model.
 *
 * Build (XC16 v1.70 container, same recipe as the M1 firmware):
 *   xc16-gcc -mcpu=33FJ32MC202 -O0 -Wall \
 *     -Wl,-T,/opt/microchip/xc16/support/dsPIC33F/gld/p33FJ32MC202.gld \
 *     -o dspic33fj32mc202-xc16-m2-pwm-qei.elf xc16-m2-pwm-qei.c
 *
 * The program configures:
 *   - T1 in 1:1 mode with a 3-instruction period and waits for T1IF;
 *   - PWM1 in edge-aligned free-running mode (P1TPER = 15), a complementary
 *     pair 1 (PWM1H1/L1 = RB14/RB15) with a 2 TCY dead time, duty 5;
 *   - QEI1 in x4 mode with index reset (QEIM = 110), POSRES = 1, MAXCNT = 199,
 *     QEA = RP0/RB0, QEB = RP1/RB1, INDX = RP2/RB2.
 * Then it writes a completion marker to 0x0E00 and loops. The integration
 * test (crates/core/tests/pic24_xc16_202_m2.rs) steps until the marker,
 * checks the register image and the PWM pads, injects quadrature edges and
 * checks POS1CNT/UPDN/QEIIF, then resets the index and checks the
 * direction-aware reset.
 *
 * All PWM/QEI pads used here exist on the 202's 28-pin package (RB0-RB15);
 * PORTC is absent and is never referenced.
 *
 * Scratch layout: results/marker at 0x0E00-0x0E04 (RAM 0x0800-0x0FFF).
 */

#include <xc.h>

#define DONE (*(volatile unsigned int *) 0x0E00)
#define WAVE (*(volatile unsigned int *) 0x0E02)

int main(void)
{
    /* ---------- T1: shared timer fabric on the 202 ---------- */
    T1CON = 0x0000;
    TMR1 = 0;
    PR1 = 3;
    T1CON = 0x8000;       /* TON, 1:1 */
    while (!IFS0bits.T1IF) {
    }

    /* ---------- PWM1: edge-aligned complementary pair 1 ---------- */
    P1TCON = 0x0000;
    P1TPER = 15;          /* period = 16 time-base ticks at 1:1 */
    P1DC1 = 5;
    P1DTCON1 = 0x0002;    /* dead time unit A = 2 TCY */
    P1DTCON2 = 0x0000;    /* both edges use unit A */
    PWM1CON1 = 0x0011;    /* PEN1H | PEN1L, PMOD1 = 0 (complementary) */
    P1OVDCON = 0x3F00;    /* generator controls every pin */
    P1FLTACON = 0x0000;
    P1TCON = 0x8000;      /* PTEN, free running, prescaler 1:1 */

    /* ---------- QEI1: x4, index reset ---------- */
    RPINR14 = 0x0100;     /* QEA1R = 0 (RP0/RB0), QEB1R = 1 (RP1/RB1) */
    RPINR15 = 0x0002;     /* INDX1R = 2 (RP2/RB2) */
    DFLT1CON = 0x0000;    /* no filter, IMV = 00 */
    MAX1CNT = 199;
    POS1CNT = 0;
    QEI1CON = 0x0604;     /* QEIM = 110 (x4, index reset), POSRES = 1 */

    WAVE = 0x0001;
    DONE = 0xBEEF;
    for (;;) {
    }
}
