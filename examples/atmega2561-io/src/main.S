; LabWired ATmega2561 minimal I/O smoke.
;
; The committed ELF (tests/fixtures/avr/atmega2561-io.elf) contains exactly this
; sequence for the shared 3-byte-PC AVR core. It sets the stack to RAMEND,
; enables USART0 transmit, writes "2561 OK\n" to UDR0, drives PB5 high through
; DDRB/PORTB, then spins. No bootloader, interrupts or EEPROM access.
;
; Each byte waits for UDRE0: data written to UDR0 while UDRE0 is clear is
; ignored by the transmitter (DS2549, USART "Sending Frames").
;
; Build (from core/):
;   avr-gcc -mmcu=atmega2561 -nostdlib -nostartfiles -Wl,--strip-all \
;     -o tests/fixtures/avr/atmega2561-io.elf examples/atmega2561-io/src/main.S

        .macro  putc ch
        ldi     r16, \ch
1:      lds     r17, 0xC0          ; UCSR0A
        sbrs    r17, 5             ; UDRE0
        rjmp    1b
        sts     0xC6, r16          ; UDR0
        .endm

        .global main
main:
        ldi     r16, 0xFF
        out     0x3D, r16          ; SPL = 0xFF
        ldi     r16, 0x21
        out     0x3E, r16          ; SPH = 0x21 -> SP = 0x21FF
        ldi     r16, 0x08
        sts     0xC1, r16          ; UCSR0B.TXEN

        putc    '2'
        putc    '5'
        putc    '6'
        putc    '1'
        putc    ' '
        putc    'O'
        putc    'K'
        putc    0x0A

        ldi     r16, 0x20
        sts     0x24, r16          ; DDRB bit 5 = output
        sts     0x25, r16          ; PORTB bit 5 = high

2:      rjmp    2b
