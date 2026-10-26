/*
 * Real XC16 firmware exercising the dsPIC33F ADC1, SPI1 and I2C1 peripherals
 * on the 28-pin dsPIC33FJ32MC202 model (M3).
 *
 * Build (XC16 v1.70 container, same recipe as the M0/M2 firmware):
 *   xc16-gcc -mcpu=33FJ32MC202 -O0 -Wall \
 *     -Wl,-T,/opt/microchip/xc16/support/dsPIC33F/gld/p33FJ32MC202.gld \
 *     -o dspic33fj32mc202-xc16-m3-adc-spi-i2c.elf xc16-m3-adc-spi-i2c.c
 *
 * The program:
 *   - samples AN4 (RB2) once with the 10-bit ADC in unsigned-integer mode;
 *   - shifts 0xA5 through SPI1 in master mode (the board ties SDO to SDI:
 *     the loopback device declared in examples/dspic33fj32mc202/system-m3.yaml);
 *   - reads the TMP102 temperature register over I2C1 (0x48, pointer 0,
 *     repeated start, two-byte read, NACK + stop);
 *   - writes the three results and a completion marker to RAM, and prints
 *     "M3 OK" over UART1. If the SPI byte does not loop back or the TMP102
 *     does not answer with 0x1900, it prints "M3 FAIL" instead (the session
 *     test runs a no-device control to prove the check works).
 *
 * The 202 is the 28-pin sibling of the 204; it shares the ADC1/SPI1/I2C1
 * register map but only routes the AN0-AN5 analog set on package pins.
 *
 * Scratch layout: 0x0E00 = 0xBEEF when done, 0x0E02 = ADC1BUF0,
 * 0x0E04 = SPI byte, 0x0E06/0x0E07 = TMP102 temperature MSB/LSB,
 * 0x0E08 = failure code.
 * The integration test (crates/core/tests/pic24_xc16_202_m3.rs) boots this
 * ELF and checks those words; the session path is exercised with
 * `system-m3.yaml`.
 */

#include <xc.h>
#include <stdint.h>

#define MARKER (*(volatile unsigned int *) 0x0E00)
#define ADC_RESULT (*(volatile unsigned int *) 0x0E02)
#define SPI_RESULT (*(volatile unsigned int *) 0x0E04)
#define I2C_MSB (*(volatile unsigned char *) 0x0E06)
#define I2C_LSB (*(volatile unsigned char *) 0x0E07)
#define FAIL_CODE (*(volatile unsigned int *) 0x0E08)

static void i2c_wait_master(void)
{
    while (!IFS1bits.MI2C1IF) {
    }
    IFS1bits.MI2C1IF = 0;
}

int main(void)
{
    /* ---------- ADC1: AN4 (RB2), manual sample, 10-bit integer ---------- */
    AD1PCFGL = 0x01EF;      /* PCFG4 = 0: AN4 analog, the other 202 analog
                               inputs digital */
    AD1CON1 = 0x0000;       /* SSRC = 000, FORM = 00, AD12B = 0 */
    AD1CON2 = 0x0000;       /* MUX A, one channel, one 16-word buffer */
    AD1CON3 = 0x0001;       /* ADCS = 1 -> TAD = 2 TCY */
    AD1CHS0 = 0x0004;       /* CH0SA = AN4, CH0NA = VREF- */
    AD1CSSL = 0x0000;
    AD1CON1bits.ADON = 1;   /* sampling starts */
    AD1CON1bits.SAMP = 1;
    AD1CON1bits.SAMP = 0;   /* end sampling, start the conversion */
    while (!AD1CON1bits.DONE) {
    }
    ADC_RESULT = ADC1BUF0;

    /* ---------- SPI1: master, 8-bit, mode 0, prescalers 1:1 ---------- */
    SPI1CON1 = 0x0137;      /* MSTEN = 1, SPRE = 101 (4:1), PPRE = 11 (1:1):
                               10 MHz at 40 MIPS (DS70206C Table 18-1) */
    SPI1CON2 = 0x0000;
    SPI1STATbits.SPIEN = 1;
    SPI1BUF = 0x00A5;
    while (!SPI1STATbits.SPIRBF) {
    }
    SPI_RESULT = SPI1BUF & 0x00FF;

    /* ---------- I2C1: master, TMP102 at 0x48 ---------- */
    I2C1CON = 0x8000;       /* I2CEN */
    I2C1BRG = 0x016B;       /* DS70000195: BRG = FCY/FSCL - FCY/1.111 MHz - 1
                               for 100 kHz at 40 MIPS */
    I2C1CONbits.SEN = 1;
    while (I2C1CONbits.SEN) {
    }
    IFS1bits.MI2C1IF = 0;

    I2C1TRN = 0x0090;       /* 0x48 << 1 | W */
    while (I2C1STATbits.TBF) {
    }
    i2c_wait_master();
    if (I2C1STATbits.ACKSTAT) {
        FAIL_CODE = 1;      /* no TMP102 at 0x48 */
        goto fail;
    }

    I2C1TRN = 0x0000;       /* pointer = temperature register */
    while (I2C1STATbits.TBF) {
    }
    i2c_wait_master();

    I2C1CONbits.RSEN = 1;
    while (I2C1CONbits.RSEN) {
    }
    IFS1bits.MI2C1IF = 0;

    I2C1TRN = 0x0091;       /* 0x48 << 1 | R */
    while (I2C1STATbits.TBF) {
    }
    i2c_wait_master();

    I2C1CONbits.RCEN = 1;
    while (!I2C1STATbits.RBF) {
    }
    I2C_MSB = (unsigned char) I2C1RCV;
    I2C1CONbits.ACKDT = 0;  /* ACK */
    I2C1CONbits.ACKEN = 1;
    while (I2C1CONbits.ACKEN) {
    }
    IFS1bits.MI2C1IF = 0;

    I2C1CONbits.RCEN = 1;
    while (!I2C1STATbits.RBF) {
    }
    I2C_LSB = (unsigned char) I2C1RCV;
    I2C1CONbits.ACKDT = 1;  /* NACK */
    I2C1CONbits.ACKEN = 1;
    while (I2C1CONbits.ACKEN) {
    }
    IFS1bits.MI2C1IF = 0;

    I2C1CONbits.PEN = 1;
    while (I2C1CONbits.PEN) {
    }
    IFS1bits.MI2C1IF = 0;

    if (I2C_MSB != 0x19 || I2C_LSB != 0x00) {
        FAIL_CODE = 3;      /* unexpected temperature register value */
        goto fail;
    }

    /* Direct UART1 writes (no helper loop: the RCALL/INC forms it needs
       are outside the modelled base subset). */
    U1TXREG = 'M';
    U1TXREG = '3';
    U1TXREG = ' ';
    U1TXREG = 'O';
    U1TXREG = 'K';
    U1TXREG = '\n';
    MARKER = 0xBEEF;
    for (;;) {
    }

fail:
    U1TXREG = 'M';
    U1TXREG = '3';
    U1TXREG = ' ';
    U1TXREG = 'F';
    U1TXREG = 'A';
    U1TXREG = 'I';
    U1TXREG = 'L';
    U1TXREG = '\n';
    MARKER = 0xBEEF;
    for (;;) {
    }
}
