#include <avr/io.h>

static void uart0_puts(const char *text) {
    while (*text != '\0') {
        UDR0 = *text++;
    }
}

int main(void) {
    DDRB |= _BV(DDB5);
    DDRB &= (uint8_t)~_BV(DDB0);
    PORTB |= _BV(PORTB0) | _BV(PORTB5);

    UCSR0B = _BV(TXEN0);
    uart0_puts("STK600 1281 READY\n");

    for (;;) {
        if ((PINB & _BV(PINB0)) == 0) {
            PORTB &= (uint8_t)~_BV(PORTB5);
        } else {
            PORTB |= _BV(PORTB5);
        }
    }
}
