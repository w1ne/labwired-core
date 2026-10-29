; LabWired ATmega2561 minimal I/O smoke.
;
; The committed ELF (tests/fixtures/avr/atmega2561-io.elf) contains exactly this
; sequence for the shared 3-byte-PC AVR core. It sets the stack to RAMEND,
; enables USART0 transmit, writes "2561 OK\n" to UDR0, drives PB5 high through
; DDRB/PORTB, then spins. No bootloader, interrupts or EEPROM access.

        ldi     r16, 0xFF
        out     0x3D, r16          ; SPL = 0xFF
        ldi     r16, 0x21
        out     0x3E, r16          ; SPH = 0x21 -> SP = 0x21FF
        ldi     r16, 0x08
        sts     0xC1, r16          ; UCSR0B.TXEN

        ldi     r16, '2'
        sts     0xC6, r16          ; UDR0
        ldi     r16, '5'
        sts     0xC6, r16
        ldi     r16, '6'
        sts     0xC6, r16
        ldi     r16, '1'
        sts     0xC6, r16
        ldi     r16, ' '
        sts     0xC6, r16
        ldi     r16, 'O'
        sts     0xC6, r16
        ldi     r16, 'K'
        sts     0xC6, r16
        ldi     r16, 0x0A
        sts     0xC6, r16

        ldi     r16, 0x20
        sts     0x24, r16          ; DDRB bit 5 = output
        sts     0x25, r16          ; PORTB bit 5 = high

1:      rjmp    1b
