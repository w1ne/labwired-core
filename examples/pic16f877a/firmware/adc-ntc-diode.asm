; LabWired - Firmware Simulation Platform
; Copyright (C) 2026 Andrii Shylenko
;
; This software is released under the MIT License.
; See the LICENSE file in the project root for full license information.
;
; PIC16F877A A/D smoke: converts AN0 (RA0) and AN1 (RA1) once each with the
; 10-bit A/D module (DS39582B section 11), keeps both results in RAM and
; prints them on the USART as "N=hhh D=hhh".
;
; 16 MHz HS crystal: FOSC/32 gives TAD = 2 us (minimum 1.6 us, Table 11-1).
; Results are right justified (ADFM = 1) against VDD/VSS (PCFG = 0000).

        list    p=16f877a
        #include <p16f877a.inc>
        errorlevel -302

        __CONFIG _FOSC_HS & _WDTE_OFF & _PWRTE_ON & _LVP_OFF

        cblock  0x20
        ntc_h                   ; 0x20: AN0 result, bits 9-8
        ntc_l                   ; 0x21: AN0 result, bits 7-0
        diode_h                 ; 0x22: AN1 result, bits 9-8
        diode_l                 ; 0x23: AN1 result, bits 7-0
        marker                  ; 0x24: 0xA5 when both results are stored
        count
        nib
        endc

        org     0x0000
        goto    start

        org     0x0005
start:
        banksel TRISC
        bcf     TRISC, 6        ; RC6/TX is an output
        movlw   d'103'          ; 9600 baud at 16 MHz with BRGH = 1
        movwf   SPBRG
        movlw   (1 << TXEN) | (1 << BRGH)
        movwf   TXSTA
        movlw   0x80            ; ADFM = 1, PCFG = 0000
        movwf   ADCON1
        banksel RCSTA
        bsf     RCSTA, SPEN

        movlw   0x81            ; FOSC/32, AN0, A/D on
        movwf   ADCON0
        call    convert
        movf    ADRESH, W
        movwf   ntc_h
        banksel ADRESL
        movf    ADRESL, W
        banksel ADCON0
        movwf   ntc_l

        movlw   0x89            ; FOSC/32, AN1, A/D on
        movwf   ADCON0
        call    convert
        movf    ADRESH, W
        movwf   diode_h
        banksel ADRESL
        movf    ADRESL, W
        banksel ADCON0
        movwf   diode_l

        movlw   0xA5
        movwf   marker

        movlw   'N'
        call    putc
        movlw   '='
        call    putc
        movf    ntc_h, W
        call    hexnib
        swapf   ntc_l, W
        call    hexnib
        movf    ntc_l, W
        call    hexnib
        movlw   ' '
        call    putc
        movlw   'D'
        call    putc
        movlw   '='
        call    putc
        movf    diode_h, W
        call    hexnib
        swapf   diode_l, W
        call    hexnib
        movf    diode_l, W
        call    hexnib
        movlw   0x0D
        call    putc
        movlw   0x0A
        call    putc
done:
        goto    done

; One conversion of the selected channel. The loop is the acquisition time:
; 27 x 3 TCY = 20.25 us at 4 MIPS (19.72 us minimum, DS39582B Equation 11-1).
convert:
        movlw   d'27'
        movwf   count
acquire:
        decfsz  count, F
        goto    acquire
        bsf     ADCON0, GO
busy:
        btfsc   ADCON0, GO
        goto    busy
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
        goto    putc
        movwf   TXREG
        return

        end
