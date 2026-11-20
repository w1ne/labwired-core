; LabWired - Firmware Simulation Platform
; Copyright (C) 2026 Andrii Shylenko
;
; This software is released under the MIT License.
; See the LICENSE file in the project root for full license information.
;
; PIC18F4520 A/D smoke: converts AN0 (RA0) and AN1 (RA1) once each with the
; 10-bit A/D module (DS39631E section 19), keeps both results in RAM and
; prints them on the EUSART as "N=hhh D=hhh".
;
; 16 MHz HS crystal: FOSC/16 gives TAD = 1 us (minimum 0.7 us, parameter 130).
; The module times the acquisition itself: ACQT = 4 TAD = 4 us (minimum
; 2.4 us, Equation 19-3). Results are right justified (ADFM = 1) against
; VDD/VSS (VCFG = 00), and PCFG = 1101 leaves AN0 and AN1 analog.

        list    p=18f4520
        #include <p18f4520.inc>
        errorlevel -302, -1301

        CONFIG  OSC = HS, WDT = OFF, PWRT = ON, LVP = OFF, PBADEN = OFF, XINST = OFF

        cblock  0x00
        ntc_h                   ; 0x000: AN0 result, bits 9-8
        ntc_l                   ; 0x001: AN0 result, bits 7-0
        diode_h                 ; 0x002: AN1 result, bits 9-8
        diode_l                 ; 0x003: AN1 result, bits 7-0
        marker                  ; 0x004: 0xA5 when both results are stored
        nib
        endc

        org     0x0000
        goto    start

        org     0x0020
start:
        bcf     TRISC, 6        ; RC6/TX is an output
        movlw   d'25'           ; 9600 baud at 16 MHz with BRGH = 1
        movwf   SPBRG
        movlw   (1 << TXEN) | (1 << BRGH)
        movwf   TXSTA
        bsf     RCSTA, SPEN

        movlw   0x0D            ; VCFG = 00, PCFG = 1101
        movwf   ADCON1
        movlw   0x95            ; ADFM = 1, ACQT = 4 TAD, FOSC/16
        movwf   ADCON2

        movlw   0x01            ; AN0, A/D on
        movwf   ADCON0
        rcall   convert
        movff   ADRESH, ntc_h
        movff   ADRESL, ntc_l

        movlw   0x05            ; AN1, A/D on
        movwf   ADCON0
        rcall   convert
        movff   ADRESH, diode_h
        movff   ADRESL, diode_l

        movlw   0xA5
        movwf   marker

        movlw   'N'
        rcall   putc
        movlw   '='
        rcall   putc
        movf    ntc_h, W
        rcall   hexnib
        swapf   ntc_l, W
        rcall   hexnib
        movf    ntc_l, W
        rcall   hexnib
        movlw   ' '
        rcall   putc
        movlw   'D'
        rcall   putc
        movlw   '='
        rcall   putc
        movf    diode_h, W
        rcall   hexnib
        swapf   diode_l, W
        rcall   hexnib
        movf    diode_l, W
        rcall   hexnib
        movlw   0x0D
        rcall   putc
        movlw   0x0A
        rcall   putc
done:
        bra     done

; One conversion of the selected channel. GO/DONE starts the programmed
; acquisition time and then the 11 TAD conversion; it clears at the end.
convert:
        bsf     ADCON0, GO
busy:
        btfsc   ADCON0, GO
        bra     busy
        return

; Print the low nibble of W as one upper-case hex digit.
hexnib:
        andlw   0x0F
        movwf   nib
        sublw   d'9'            ; C = 1 when the nibble is 0-9
        movf    nib, W
        btfss   STATUS, C
        addlw   7
        addlw   '0'
putc:
        btfss   PIR1, TXIF
        bra     putc
        movwf   TXREG
        return

        end
