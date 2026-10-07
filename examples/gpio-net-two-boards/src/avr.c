/* ATmega328P side of the gpio-net-two-boards example (avr-libc, no Arduino
 * core). Wires to the STM32 board:
 *
 *   irq    PD2  output, push-pull        -> STM32 PB0
 *   ready  PD3  input, INT1 (rising)     <- STM32 PB1
 *   alert  PD4  open-drain by DDR, shared, 10k pull-up, PCINT20
 *                                        <-> STM32 PB4
 *
 * Edges are counted by interrupts: INT1 on the rising edges of ready, and the
 * PCINT2 pin-change interrupt on alert (the handler reads PIND to tell a fall
 * from a rise). Between edges the core sleeps in idle mode, so the simulator
 * can skip the waits instead of running a polling loop.
 *
 * Sequence:
 *   1. put 10 pulses on irq;
 *   2. sleep until INT1 has counted 7 rising edges on ready;
 *   3. sleep until PCINT2 has counted 5 falling and 5 rising edges on alert
 *      (the STM32 pulls it);
 *   4. pull alert low 3 times (DDR high = drive the PORT bit, 0; DDR low =
 *      release), the STM32 counts them;
 *   5. report over USART0.
 */
#define F_CPU 16000000UL
#include <avr/interrupt.h>
#include <avr/io.h>
#include <avr/sleep.h>
#include <util/delay.h>
#include <stdint.h>

/* The browser lab runs the same firmware 20x slower (TIME_SCALE=20, see
 * build.sh) so a person can watch the pulses; every count is unchanged. */
#ifndef TIME_SCALE
#define TIME_SCALE 1
#endif
#define IRQ   (1u << PD2)
#define READY (1u << PD3)
#define ALERT (1u << PD4)

static volatile uint8_t ready, a_fall, a_rise, alert_prev;

ISR(INT1_vect) { ++ready; }

ISR(PCINT2_vect) {
    uint8_t cur = PIND & ALERT;
    if (cur == alert_prev) return;   /* another PORTD pad moved */
    if (cur) ++a_rise; else ++a_fall;
    alert_prev = cur;
}

/* Sleep until `cond` holds. SEI's next instruction runs before any
 * interrupt, so an edge between the check and SLEEP still wakes the core. */
#define SLEEP_UNTIL(cond)            \
    do {                             \
        cli();                       \
        while (!(cond)) {            \
            sei();                   \
            sleep_cpu();             \
            cli();                   \
        }                            \
        sei();                       \
    } while (0)

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
    UBRR0 = 103;                         /* 16 MHz / 9600 */
    UCSR0B = (1u << TXEN0);
    PORTD &= (uint8_t)~(IRQ | ALERT);    /* irq low, alert PORT bit 0 */
    DDRD |= IRQ;                         /* alert (DDR bit 0) is released */
    _delay_ms(1);                        /* let the STM32 arm EXTI */

    EICRA = (1u << ISC11) | (1u << ISC10);   /* INT1 on rising edges */
    EIFR = 1u << INTF1;
    EIMSK = 1u << INT1;
    alert_prev = PIND & ALERT;
    PCMSK2 = 1u << PCINT20;                  /* PD4 */
    PCIFR = 1u << PCIF2;
    PCICR = 1u << PCIE2;
    set_sleep_mode(SLEEP_MODE_IDLE);
    sleep_enable();
    sei();

    for (uint8_t i = 0; i < 10; ++i) {
        PORTD |= IRQ;
        _delay_us(20 * TIME_SCALE);
        PORTD &= (uint8_t)~IRQ;
        _delay_us(20 * TIME_SCALE);
    }

    SLEEP_UNTIL(ready >= 7);
    SLEEP_UNTIL(a_fall >= 5 && a_rise >= 5);
    /* Our own pulls below would count too: stop listening. */
    PCICR = 0;
    EIMSK = 0;
    sleep_disable();

    _delay_us(100 * TIME_SCALE);
    for (uint8_t i = 0; i < 3; ++i) {
        DDRD |= ALERT;                   /* drive low */
        _delay_us(30 * TIME_SCALE);
        DDRD &= (uint8_t)~ALERT;         /* release */
        _delay_us(30 * TIME_SCALE);
    }

    put("AVR ready=");
    put_num(ready);
    put(" alert f=");
    put_num(a_fall);
    put(" r=");
    put_num(a_rise);
    put("\n");
    for (;;) {}
}
