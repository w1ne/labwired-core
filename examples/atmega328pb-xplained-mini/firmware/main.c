#include <stdint.h>

/* ATmega328PB data-space addresses from Microchip DS40001906C. */
#define DATA_REG(address) (*(volatile uint8_t *)(address))
#define DDRB DATA_REG(0x24)
#define PORTB DATA_REG(0x25)
#define UCSR0A DATA_REG(0xC0)
#define UCSR0B DATA_REG(0xC1)
#define UCSR0C DATA_REG(0xC2)
#define UBRR0L DATA_REG(0xC4)
#define UBRR0H DATA_REG(0xC5)
#define UDR0 DATA_REG(0xC6)
#define BIT(n) (1u << (n))

static void usart0_putc(uint8_t byte) {
    while ((UCSR0A & BIT(5)) == 0) {
    }
    UDR0 = byte;
}

static void usart0_write(const char *text) {
    while (*text != '\0') {
        usart0_putc((uint8_t)*text++);
    }
}

static void delay(void) {
    for (volatile uint16_t i = 0; i < 1000; ++i) {
        __asm__ __volatile__("nop");
    }
}

/* AVR-LibC on the local toolchain has no ATmega328PB startup object. */
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

    UCSR0A = BIT(1); /* U2X0: 9600 baud at 16 MHz. */
    UBRR0H = 0;
    UBRR0L = 103;
    UCSR0C = BIT(2) | BIT(1); /* 8 data bits, no parity, one stop bit. */
    UCSR0B = BIT(4) | BIT(3); /* RXEN0 | TXEN0. */
    usart0_write("XPLAINED_MINI_OK\n");

    for (;;) {
        PORTB ^= BIT(5);
        delay();
    }
}
