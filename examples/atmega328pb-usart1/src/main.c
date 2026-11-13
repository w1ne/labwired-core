#include <stdint.h>

/* ATmega328PB data-space addresses from the device data sheet. */
#define DATA_REG(address) (*(volatile uint8_t *)(address))
#define DDRB DATA_REG(0x24)
#define PORTB DATA_REG(0x25)
#define UCSR1A DATA_REG(0xC8)
#define UCSR1B DATA_REG(0xC9)
#define UCSR1C DATA_REG(0xCA)
#define UBRR1L DATA_REG(0xCC)
#define UBRR1H DATA_REG(0xCD)
#define UDR1 DATA_REG(0xCE)
#define BIT(n) (1u << (n))

static void usart1_putc(uint8_t byte) {
    while ((UCSR1A & BIT(5)) == 0) {
    }
    UDR1 = byte;
}

/* The host AVR-LibC lacks the PB startup object, so this smoke provides reset. */
__attribute__((naked, used, section(".vectors"))) void __vectors(void) {
    __asm__ __volatile__(
        "ldi r16, 0x08\n\t"
        "out 0x3e, r16\n\t" /* SPH = RAMEND[15:8] */
        "ldi r16, 0xff\n\t"
        "out 0x3d, r16\n\t" /* SPL = RAMEND[7:0] */
        "rjmp main\n\t");
}

int main(void) {
    DDRB |= BIT(5);
    PORTB |= BIT(5);

    UCSR1A = BIT(1); /* U2X1: 9600 baud at 16 MHz. */
    UBRR1H = 0;
    UBRR1L = 103;
    UCSR1C = BIT(2) | BIT(1); /* UCSZ11:0: 8 data bits. */
    UCSR1B = BIT(4) | BIT(3); /* RXEN1 | TXEN1. */

    usart1_putc('P');
    usart1_putc('B');
    usart1_putc('_');
    usart1_putc('U');
    usart1_putc('S');
    usart1_putc('A');
    usart1_putc('R');
    usart1_putc('T');
    usart1_putc('1');
    usart1_putc('_');
    usart1_putc('O');
    usart1_putc('K');
    usart1_putc('\n');

    for (;;) {
    }
}
