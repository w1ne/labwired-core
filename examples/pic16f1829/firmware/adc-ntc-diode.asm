; LabWired - Firmware Simulation Platform
; Copyright (C) 2026 Andrii Shylenko
;
; This software is released under the MIT License.
; See the LICENSE file in the project root for full license information.
;
; PIC16F1829 ADC smoke for the Curiosity board (DM164137): converts AN4
; (RC0, the POT1 net), AN6 (RC2) and the internal FVR Buffer1 once each with
; the 10-bit ADC (DS40001440E section 16), keeps the results in common RAM
; and prints them on the EUSART as "N=hhh D=hhh F=hhh".
;
; 16 MHz HFINTOSC: FOSC/16 gives TAD = 1.0 us (Table 16-1). Results are right
; justified (ADFM = 1) against VDD/VSS (ADPREF = 00, ADNREF = 0). The FVR is
; on at 1.024 V (FVRCON = FVREN | ADFVR 01), so its code is 1024 * 1.024 / VDD.

        list    p=16f1829
        #include <p16f1829.inc>
        errorlevel -302

        __CONFIG _CONFIG1, _FOSC_INTOSC & _WDTE_OFF & _PWRTE_ON & _MCLRE_ON
        __CONFIG _CONFIG2, _PLLEN_OFF & _LVP_OFF

        cblock  0x70            ; common RAM, the same in every bank
        ntc_h                   ; 0x70: AN4 result, bits 9-8
        ntc_l                   ; 0x71: AN4 result, bits 7-0
        diode_h                 ; 0x72: AN6 result, bits 9-8
        diode_l                 ; 0x73: AN6 result, bits 7-0
        fvr_h                   ; 0x74: FVR Buffer1 result, bits 9-8
        fvr_l                   ; 0x75: FVR Buffer1 result, bits 7-0
        marker                  ; 0x76: 0xA5 when all results are stored
        count
        nib
        hi
        lo
        endc

        org     0x0000
        goto    start

        org     0x0005
start:
        banksel OSCCON
        movlw   0x78            ; IRCF = 1111: 16 MHz HFINTOSC
        movwf   OSCCON
        banksel SPBRGL
        movlw   d'103'          ; 9600 baud at 16 MHz with BRGH = 1
        movwf   SPBRGL
        movlw   (1 << TXEN) | (1 << BRGH)
        movwf   TXSTA
        bsf     RCSTA, SPEN     ; TX is RB7
        banksel FVRCON
        movlw   0x81            ; FVREN = 1, ADFVR = 01 (1.024 V)
        movwf   FVRCON
        banksel ADCON1          ; ADCON0 and ADRESH:ADRESL are in this bank
        movlw   0xD0            ; ADFM = 1, FOSC/16, VDD/VSS
        movwf   ADCON1

        movlw   0x11            ; AN4, ADC on
        call    convert
        movf    ADRESH, W
        movwf   ntc_h
        movf    ADRESL, W
        movwf   ntc_l

        movlw   0x19            ; AN6, ADC on
        call    convert
        movf    ADRESH, W
        movwf   diode_h
        movf    ADRESL, W
        movwf   diode_l

        movlw   0x7D            ; FVR Buffer1 (CHS = 11111), ADC on
        call    convert
        movf    ADRESH, W
        movwf   fvr_h
        movf    ADRESL, W
        movwf   fvr_l

        movlw   0xA5
        movwf   marker

        movlw   'N'
        call    putc
        movf    ntc_h, W
        movwf   hi
        movf    ntc_l, W
        movwf   lo
        call    print3
        movlw   ' '
        call    putc
        movlw   'D'
        call    putc
        movf    diode_h, W
        movwf   hi
        movf    diode_l, W
        movwf   lo
        call    print3
        movlw   ' '
        call    putc
        movlw   'F'
        call    putc
        movf    fvr_h, W
        movwf   hi
        movf    fvr_l, W
        movwf   lo
        call    print3
        movlw   0x0D
        call    putc
        movlw   0x0A
        call    putc
done:
        goto    done

; One conversion of the channel in W (an ADCON0 value). Call and return in
; the ADCON0 bank. The loop is the acquisition time: 27 x 3 TCY = 20.25 us at
; 4 MIPS (5 us typical, DS40001440E parameter AD132).
convert:
        movwf   ADCON0
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

; Print "=" and the three hex digits of hi:lo.
print3:
        movlw   '='
        call    putc
        movf    hi, W
        call    hexnib
        swapf   lo, W
        call    hexnib
        movf    lo, W

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
        banksel PIR1
        btfss   PIR1, TXIF
        goto    $-1
        banksel TXREG
        movwf   TXREG
        return

        end
