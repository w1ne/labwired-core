/* PIC16F1829 smoke: EUSART TX banner + LED latch on RA5 (LED D4 on the
 * Curiosity 8-bit board, DM164137, with a PIC16F1829 in its 20-pin socket).
 *
 * Fosc = 16 MHz HFINTOSC (OSCCON.IRCF = 1111, SCS = 1x), 9600 baud
 * (BRGH = 1, BRG16 = 0, SPBRGL = 103; DS40001440E Table 26-5). TX is on
 * RB7 (APFCON0.TXCKSEL = 0, the reset default). Built with SDCC 4.4.0
 * (pic14 port, enhanced mid-range) + gputils 1.4.0; see build.sh.
 */
#include <pic16regs.h>

/* Internal oscillator, WDT off, power-up timer on, MCLR on RA3. */
static __code unsigned int __at (_CONFIG1) config1 = _FOSC_INTOSC & _WDTE_OFF & _PWRTE_ON & _MCLRE_ON;
/* Stack overflow/underflow reset on, LVP off. */
static __code unsigned int __at (_CONFIG2) config2 = _STVREN_ON & _LVP_OFF;

static const char banner[] = "PIC16F1829 OK\r\n";

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

static volatile unsigned char ticks, tmr0_ticks;

/* INT (raised in software by setting INTF) and TMR0 overflow. The enhanced
 * core saves W/STATUS/BSR/PCLATH/FSRs in hardware, so SDCC emits no
 * context-save prologue: this ISR clobbers all of them and relies on
 * RETFIE restoring main's context. */
static void isr(void) __interrupt
{
    if (INTF) {
        INTF = 0;
        ticks++;
    }
    if (TMR0IF) {
        TMR0IF = 0;
        tmr0_ticks++;
    }
}

static const char *name(unsigned char n)
{
    switch (n) {
    case 0: return "zero";
    case 1: return "one";
    case 2: return "two";
    case 3: return "three";
    case 4: return "four";
    default: return "many";
    }
}

static unsigned int mul16(unsigned int a, unsigned int b)
{
    return a * b;
}

static unsigned char sum(const unsigned char *p, unsigned char n)
{
    unsigned char s = 0;
    while (n--)
        s += *p++;
    return s;
}

void main(void)
{
    unsigned char i;
    unsigned char buf[8];
    static volatile unsigned int a, b;
    unsigned char errors;

    OSCCON = 0x7A;            /* 16 MHz HFINTOSC */
    while (!HFIOFR)
        ;

    TRISB7 = 0;
    SPBRGL = 103;
    SPBRGH = 0;
    BAUDCON = 0;
    TXSTA = 0x24;             /* TXEN | BRGH */
    RCSTA = 0x80;             /* SPEN */

    TRISA5 = 0;
    LATA5 = 1;

    uart_puts(banner);
    uart_puts(RA5 ? "LED=1\r\n" : "LED=0\r\n");

    for (i = 0; i < 3; i++)
        LATA5 ^= 1;
    uart_puts(RA5 ? "LED=0?\r\n" : "TOGGLE OK\r\n");

    /* Stack RAM through FSR + linear arithmetic. */
    for (i = 0; i < sizeof(buf); i++)
        buf[i] = i * 3 + 1;
    uart_puts(sum(buf, sizeof(buf)) == 92 ? "SUM OK\r\n" : "SUM BAD\r\n");

    /* Three software interrupts; the ISR runs between main's instructions
     * and must leave BSR/W/FSR exactly as main had them. */
    INTE = 1;
    GIE = 1;
    for (i = 0; i < 3; i++) {
        INTF = 1;
        while (ticks == i)
            ;
    }
    GIE = 0;
    uart_puts(ticks == 3 ? "IRQ OK\r\n" : "IRQ BAD\r\n");

    uart_puts(name(3));
    uart_puts(" ");
    uart_puts(name(9));
    uart_puts("\r\n");

    /* TMR0 at Fosc/4 with no prescaler overflows every 256 cycles, so its
     * interrupts land at arbitrary points in this loop. Any context the
     * ISR failed to restore shows up as a wrong result. */
    OPTION_REG = 0x08;        /* TMR0CS = 0 (Fosc/4), PSA = 1 */
    TMR0 = 0;
    TMR0IF = 0;
    TMR0IE = 1;
    GIE = 1;
    errors = 0;
    while (tmr0_ticks < 40) {
        for (i = 0; i < sizeof(buf); i++)
            buf[i] = i * 3 + 1;
        if (sum(buf, sizeof(buf)) != 92)
            errors++;
        a = 1234;
        if (mul16(a, 53) != 65402u)
            errors++;
        if (name(4)[1] != 'o')
            errors++;
    }
    GIE = 0;
    uart_puts(errors == 0 ? "TMR0 IRQ OK\r\n" : "TMR0 IRQ BAD\r\n");

    /* 1234 * 53 = 65402; 300 * 300 = 90000 = 24464 mod 2^16. */
    a = 1234;
    b = 300;
    uart_puts(mul16(a, 53) == 65402u && mul16(b, b) == 24464u ? "MUL OK\r\n" : "MUL BAD\r\n");

    for (;;)
        ;
}
