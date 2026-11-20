; MSP430 instruction-set exercise for the GDB-simulator lockstep
; (crates/core/tests/msp430_gdb_sim_lockstep.rs). Every instruction and
; addressing mode of the 16-bit CPU, and with __MSP430X__ the CPUX additions.
; The simulator records all sixteen registers after each instruction, so the
; code only has to execute the cases; it ends in `finished`, a jump to itself.

        .text
        .global _start
_start:
        mov     #__stack, sp
        mov     #0x5a80, &0x0120        ; WDTCTL = WDTPW | WDTHOLD
        clr     r12

; ---- format I and II over every operand pair -----------------------------
        mov     #vals, r10
outer:  mov     @r10+, r4
        mov     #vals, r11
inner:  mov     @r11+, r5
        mov     r4, r6
        add     r5, r6
        mov     r4, r6
        addc    r5, r6
        mov     r4, r6
        sub     r5, r6
        mov     r4, r6
        subc    r5, r6
        cmp     r5, r4
        jne     1f
        inc     r12
1:      jeq     1f
        inc     r12
1:      jnc     1f
        inc     r12
1:      jc      1f
        inc     r12
1:      jn      1f
        inc     r12
1:      jge     1f
        inc     r12
1:      jl      1f
        inc     r12
1:      bit     r5, r4
        mov     r4, r6
        bic     r5, r6
        mov     r4, r6
        bis     r5, r6
        mov     r4, r6
        xor     r5, r6
        mov     r4, r6
        and     r5, r6
        mov     r4, r6
        add.b   r5, r6
        mov     r4, r6
        addc.b  r5, r6
        mov     r4, r6
        sub.b   r5, r6
        mov     r4, r6
        subc.b  r5, r6
        cmp.b   r5, r4
        jge     1f
        inc     r12
1:      jl      1f
        inc     r12
1:      bit.b   r5, r4
        mov     r4, r6
        bic.b   r5, r6
        mov     r4, r6
        bis.b   r5, r6
        mov     r4, r6
        xor.b   r5, r6
        mov     r4, r6
        and.b   r5, r6
        mov.b   r4, r6
        mov     r5, r6
        rrc     r6
        rra     r6
        swpb    r6
        sxt     r6
        rrc.b   r6
        rra.b   r6
        cmp     #vals_end, r11
        jne     inner
        cmp     #vals_end, r10
        jne     outer

; ---- decimal add ----------------------------------------------------------
        mov     #bcd, r10
1:      mov     @r10+, r4
        mov     @r10+, r5
        clrc
        dadd    r4, r5
        dadd    r4, r5
        setc
        dadd.b  r4, r5
        dadd.b  r4, r5
        cmp     #bcd_end, r10
        jne     1b

; ---- constant generators --------------------------------------------------
        mov     #0, r6
        add     #1, r6
        add     #2, r6
        add     #4, r6
        add     #8, r6
        add     #-1, r6
        mov.b   #-1, r6
        add.b   #1, r6
        add.b   #2, r6
        add.b   #4, r6
        add.b   #8, r6
        mov     #0x1234, r6
        mov.b   #0x56, r6

; ---- source and destination addressing modes ------------------------------
; Symbolic operands use `symw` and nothing else does: the GDB simulator adds
; the PC-relative index without wrapping at 64 KiB, so on it they land in a
; different cell from the absolute and indexed operands.
        mov     #scratch, r10
        mov     #0x1111, 0(r10)
        mov     #0x2222, 2(r10)
        mov     #0x3333, &scratch+4
        mov     #0x4444, &scratch+6
        mov     #0x4444, symw
        mov     0(r10), r6
        mov     2(r10), r7
        mov     &scratch+4, r8
        mov     symw, r9
        mov     @r10, r6
        mov     @r10+, r7
        mov     @r10+, r8
        mov.b   @r10+, r9
        mov.b   @r10+, r9
        add     r6, 0(r10)
        add     @r10, 2(r10)
        add     -4(r10), &scratch+8
        add     &scratch, symw+2
        add     symw+2, -2(r10)
        add.b   #0x7f, 1(r10)
        addc.b  #0xff, &scratch+1
        sub     #1, 0(r10)
        subc    #0, 2(r10)
        xor     #-1, 0(r10)
        and     #0x0ff0, 2(r10)
        bis.b   #0x81, &scratch+3
        bic.b   #0x01, symw+1
        bit     #0x8000, 0(r10)
        cmp     #0, 2(r10)
        cmp.b   @r10, 1(r10)
        mov     #0x0450, 8(r10)         ; valid BCD: other digits are undefined
        dadd    #0x0199, 8(r10)
        dadd.b  #0x99, 8(r10)
        rrc     0(r10)
        rra     2(r10)
        rrc.b   &scratch+1
        rra.b   symw+1
        swpb    0(r10)
        sxt     2(r10)
        swpb    &scratch+4
        sxt     symw+2
        inv     r6
        inc     r6
        incd    r6
        dec     r6
        decd    r6
        tst     r6
        rla     r6
        rlc     r6
        adc     r6
        sbc     r6
        dadc    r6
        clrn
        setn
        clrz
        setz
        clrc
        setc
        nop

; ---- stack ----------------------------------------------------------------
        push    r4
        push    #0x1234
        push    #8
        push    @r10
        push    2(r10)
        push    &scratch
        push    symw+2
        push.b  r5
        push.b  #0x7f
        pop     r6
        pop     r7                      ; (`pop.b`: unit tests; the GDB simulator
                                        ; steps SP by 1 for a byte `@sp+`)
        pop     r8
        pop     r9
        pop     r6
        pop     r7
        pop     r8
        pop     r9
        push    r4
        mov     2(sp), r6
        mov     @sp, r7
        mov     @sp+, r8
        mov     sp, r9
        add     #2, sp
        sub     #2, sp
        mov     @sp+, r6

; ---- calls ----------------------------------------------------------------
        call    #leaf
        mov     #leaf, r6
        call    r6
        mov     #ptrs, r10
        call    @r10
        call    @r10+
        call    -2(r10)
        call    &ptrs
        ; `call ptrs+2` (symbolic) is left to the unit tests: the GDB simulator
        ; adds the PC-relative index without the 64 KiB wrap of SLAU144 4.4.2.
        mov     #leaf, r6
        br      #1f
        inc     r12
1:      mov     #2f, pc
        inc     r12
2:      add     #4, pc
        inc     r12
        inc     r12
        mov     pc, r6
        mov     #9f, r6
        push    r6
        push    #0x0007                 ; N, Z, C for the frame
        reti
        inc     r12
9:
#ifdef __MSP430X__
        calla   #cpux
#endif
finished:
        jmp     finished

leaf:   add     #3, r12
        ret
leaf2:  add     #5, r12
        ret

#ifdef __MSP430X__
; ---- CPUX -----------------------------------------------------------------
cpux:
        mova    #wide, r10
xouter: movx.a  @r10+, r4
        mova    #wide, r11
xinner: mova    @r11+, r5
        mova    r4, r6
        addx.a  r5, r6
        mova    r4, r6
        addcx.a r5, r6
        mova    r4, r6
        subx.a  r5, r6
        mova    r4, r6
        subcx.a r5, r6
        cmpx.a  r5, r4
        jge     1f
        inc     r12
1:      jl      1f
        inc     r12
1:      jc      1f
        inc     r12
1:      bitx.a  r5, r4
        mova    r4, r6
        bicx.a  r5, r6
        mova    r4, r6
        bisx.a  r5, r6
        mova    r4, r6
        xorx.a  r5, r6
        mova    r4, r6
        andx.a  r5, r6
        mova    r4, r6
        cmpa    r5, r6
        adda    r5, r6
        suba    r5, r6
        suba    r5, r6
        ; 16-bit and 8-bit instructions clear the upper register bits
        mova    r4, r6
        add     r5, r6
        mova    r4, r6
        add.b   r5, r6
        mova    r4, r6
        mov     r6, r6
        mova    r4, r6
        addx.w  r5, r6
        mova    r4, r6
        addx.b  r5, r6
        mova    r4, r6
        swpb    r6
        mova    r4, r6
        sxt     r6
        mova    r4, r6
        swpbx.a r6
        mova    r4, r6
        sxtx.a  r6
        mova    r4, r6
        rrcx.a  r6
        rrax.a  r6
        rrux.a  r6
        rrcx.w  r6
        rrax.w  r6
        rrux.w  r6
        mova    r4, r6
        rrcx.b  r6
        rrax.b  r6
        rrux.b  r6
        mova    r4, r6
        rrcm.a  #1, r6
        rram.a  #2, r6
        rlam.a  #3, r6
        rrum.a  #4, r6
        mova    r4, r6
        rrcm.w  #4, r6
        mova    r4, r6
        rram.w  #3, r6
        mova    r4, r6
        rlam.w  #2, r6
        mova    r4, r6
        rrum.w  #1, r6
        mova    r4, r6
        rpt     #3 { rrax.a r6
        rpt     #5 { rlax.a r6
        rpt     #2 { rrcx.a r6
        rpt     #4 { rrux.w r6
        mov     #6, r7
        rpt     r7 { rlax.a r6
        rpt     #3 { addx.a r5, r6
        cmpa    #wide_end, r11
        jne     xinner
        cmpa    #wide_end, r10
        jne     xouter

        ; `daddx.a` is left to the unit tests: the GDB simulator adds four
        ; decimal digits only.

        ; 20-bit immediates and memory operands
        movx.a  #0xabcde, r6
        addx.a  #0x12345, r6
        subx.a  #0xfffff, r6
        mova    #scratch, r10
        movx.a  r6, 0(r10)
        movx.a  #0x54321, 4(r10)
        movx.a  r6, &scratch+8
        movx.a  0(r10), r7
        movx.a  4(r10), r8
        movx.a  &scratch+8, r9
        movx.a  scratch+4, r9
        addx.a  @r10, r7
        addx.a  @r10+, r8
        addx.a  r7, 0(r10)
        addx.a  #0x80000, &scratch+8
        addcx.a #0, &scratch+4
        subx.w  #1, 0(r10)
        addx.b  #0xff, 1(r10)
        xorx.a  #-1, 0(r10)
        rrcx.a  0(r10)
        rrax.a  &scratch+8
        rrcx.w  0(r10)
        rrax.b  0(r10)
        swpbx.a 0(r10)
        swpbx.w 0(r10)
        sxtx.a  0(r10)
        sxtx.w  &scratch+8
        mova    #scratch, r10
        mova    @r10, r6
        mova    @r10+, r7
        mova    4(r10), r8
        mova    &scratch+8, r9
        mova    r6, &scratch+12
        mova    r7, 12(r10)
        mova    -4(r10), r8
        mova    #0xfedcb, r6
        cmpa    #0xfedcb, r6
        adda    #0x00235, r6
        suba    #0x70000, r6
        cmpa    #0, r6

        ; a 16-bit index wraps at 64 KiB below it and sign-extends above
        mova    #hidata, r10
        mov     2(r10), r6
        mov     -2(r10), r7
        mov     @r10+, r8
        mov.b   @r10+, r9
        movx.a  hidata, r6
        mov     #0x7777, &0x1200
        mov     #0xfff0, r10
        mov     0x1210(r10), r7

        ; stack
        mova    #0x9abcd, r4
        pushx.a r4
        pushx.a #0x12345
        pushx.w r4
        pushx.b r4
        popx.w  r6
        popx.w  r7
        popx.a  r8
        popx.a  r9
        mova    #0x11111, r6
        mova    #0x22222, r7
        mova    #0x33333, r8
        pushm.a #3, r8
        clr     r6
        clr     r7
        clr     r8
        popm.a  #3, r8
        pushm.w #2, r8
        popm.w  #2, r9
        pushm.a #1, r4
        popm.a  #1, r5

        ; calls and returns across the 64 KiB boundary
        calla   #far
        mova    #far, r6
        calla   r6
        mova    #farptr, r10
        calla   @r10
        calla   @r10+
        ; a negative CALLA index is left to the unit tests: the GDB simulator
        ; zero-extends it.
        mova    #farptr-4, r10
        calla   4(r10)
        calla   &farptr
        ; RETI with a 20-bit frame: PC bits 19:16 ride in SR bits 15:12
        mova    #far2, r6
        push    r6
        mova    r6, r7
        rrum.a  #4, r7
        rrum.a  #4, r7
        rrum.a  #4, r7
        rrum.a  #4, r7
        rlam.a  #4, r7
        rlam.a  #4, r7
        rlam.a  #4, r7
        bis     #0x0003, r7
        push    r7
        reti
        inc     r12
back:   reta

        .section .upper.text, "ax"
far:    adda    #7, r12
        reta
far2:   adda    #9, r12
        bra     #back
#endif

        .section .rodata
vals:   .word   0x0000, 0x0001, 0x7fff, 0x8000, 0xffff, 0x00ff, 0x0080, 0x1234
        .word   0xa5a5, 0x5a5a, 0xff00, 0x007f
vals_end:
bcd:    .word   0x0000, 0x0000, 0x1234, 0x5678, 0x9999, 0x0001, 0x4950, 0x5050
        .word   0x0099, 0x0001, 0x9876, 0x0125
bcd_end:
ptrs:   .word   leaf, leaf2
#ifdef __MSP430X__
wide:   .long   0x00000, 0x00001, 0x7ffff, 0x80000, 0xfffff, 0x0ffff, 0x10000
        .long   0x12345, 0xa5a5a, 0x08000
wide_end:
farptr: .long   far
        .section .upper.rodata, "a"
        .word   0x1357
hidata: .word   0x2468, 0x369c, 0x48d0
#endif

        .section .bss
scratch:
        .space  16
symw:   .space  8

        .section .resetvec, "a"
        .word   _start
