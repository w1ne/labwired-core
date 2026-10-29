#include <avr/io.h>

int main(void) {
    DDRB |= _BV(DDB5);
    PORTB |= _BV(PORTB5);

    UCSR0B = _BV(TXEN0);
    UDR0 = '1';
    UDR0 = '2';
    UDR0 = '8';
    UDR0 = '1';
    UDR0 = ' ';
    UDR0 = 'O';
    UDR0 = 'K';
    UDR0 = '\n';

    for (;;) {
    }
}
