/* ATmega328P side of the gpio-net-f1-f4 example (avr-libc, no Arduino core).
 *
 *   irq    PD2  output, push-pull             -> F103 PB0 and F401 PC1
 *   alert  PD4  open-drain by DDR (PORT bit 0) <-> F401 PA8 (internal pull-up)
 *   wake   PD5  input, INTERNAL pull-up (PORT bit 1, DDR bit 0) <- F103 PA0
 *
 * Sequence: 10 pulses on irq; 3 low pulses on alert; then count the F103's
 * 4 low pulses on wake by polling PIND and report over USART0.
 */
#define F_CPU 16000000UL
#include <avr/io.h>
#include <util/delay.h>
#include <stdint.h>

#define IRQ   (1u << PD2)
#define ALERT (1u << PD4)
#define WAKE  (1u << PD5)

static void put(const char *s) {
    while (*s) {
        while (!(UCSR0A & (1u << UDRE0))) {}
        UDR0 = (uint8_t)*s++;
    }
}

static void put_num(uint8_t v) {
    char b[4];
    uint8_t n = 0;
    do { b[n++] = (char)('0' + v % 10u); v /= 10u; } while (v);
    while (n) {
        while (!(UCSR0A & (1u << UDRE0))) {}
        UDR0 = (uint8_t)b[--n];
    }
}

int main(void) {
    UBRR0 = 8;                           /* 16 MHz / 115200 */
    UCSR0B = (1u << TXEN0);
    PORTD = WAKE;                        /* wake pull-up on; irq, alert low */
    DDRD = IRQ;                          /* alert (DDR bit 0) is released */
    _delay_ms(1);                        /* let both STM32s arm EXTI */

    for (uint8_t i = 0; i < 10; ++i) {
        PORTD |= IRQ;
        _delay_us(20);
        PORTD &= (uint8_t)~IRQ;
        _delay_us(20);
    }

    _delay_us(100);
    for (uint8_t i = 0; i < 3; ++i) {
        DDRD |= ALERT;                   /* drive low */
        _delay_us(30);
        DDRD &= (uint8_t)~ALERT;         /* release: the F401 pull-up lifts it */
        _delay_us(30);
    }

    uint8_t fall = 0, rise = 0, prev = PIND & WAKE;
    while (fall < 4 || rise < 4) {
        uint8_t cur = PIND & WAKE;
        if (prev && !cur) ++fall;
        if (!prev && cur) ++rise;
        prev = cur;
    }

    put("AVR wake f=");
    put_num(fall);
    put(" r=");
    put_num(rise);
    put("\n");
    for (;;) {}
}
