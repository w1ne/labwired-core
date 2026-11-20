@ RP2040 PIO0 stalled-chain fixture (crates/cli/tests/pio_stalled_chain.rs).
@ SM0 runs `pull block; out pins, 8; jmp 0`, so it blocks on the empty TX FIFO
@ between words. Each trial pads a different number of cycles (a block that
@ sleeps while stalled must keep the clock divider's phase), writes a word to
@ TXF0 and counts poll iterations until PINS shows it. The poll count is the
@ write-to-pins latency, so a divider phase that is off by one cycle after
@ the sleep shifts at least one trial. Three dividers: 1.0, 3.5 and 135.25.
    .syntax unified
    .cpu cortex-m0plus
    .thumb

    .section .vectors, "a"
    .word 0x20000400
    .word reset

    .text
    .thumb_func
    .global reset
reset:
    ldr r7, =0x50200000         @ PIO0
    ldr r6, =0x40034000         @ UART0 DR
    ldr r5, =0x502000c8         @ PIO0 SM0_CLKDIV (PINCTRL at +0x14)
    ldr r0, =0x80A0             @ pull block
    str r0, [r7, #0x48]
    ldr r0, =0x6008             @ out pins, 8
    str r0, [r7, #0x4c]
    movs r0, #0                 @ jmp 0
    str r0, [r7, #0x50]
    ldr r0, =0x00800000         @ PINCTRL: OUT_COUNT = 8, OUT_BASE = 0
    str r0, [r5, #0x14]
    movs r0, #1
    str r0, [r7, #0x00]         @ CTRL: SM0 on
    ldr r0, =300
1:  subs r0, r0, #1
    bne 1b
    movs r3, #65
    str r3, [r7, #0x10]         @ TXF0: SM0 pulls it and outs it to the pins
    movs r2, #0
2:  adds r2, r2, #1
    ldr r0, [r7, #0x3c]         @ PINS (pin outputs)
    cmp r0, r3
    bne 2b
    str r2, [r6]                @ polls until the pins changed
    ldr r0, =337
1:  subs r0, r0, #1
    bne 1b
    nop
    movs r3, #66
    str r3, [r7, #0x10]         @ TXF0: SM0 pulls it and outs it to the pins
    movs r2, #0
2:  adds r2, r2, #1
    ldr r0, [r7, #0x3c]         @ PINS (pin outputs)
    cmp r0, r3
    bne 2b
    str r2, [r6]                @ polls until the pins changed
    ldr r0, =374
1:  subs r0, r0, #1
    bne 1b
    nop
    nop
    movs r3, #67
    str r3, [r7, #0x10]         @ TXF0: SM0 pulls it and outs it to the pins
    movs r2, #0
2:  adds r2, r2, #1
    ldr r0, [r7, #0x3c]         @ PINS (pin outputs)
    cmp r0, r3
    bne 2b
    str r2, [r6]                @ polls until the pins changed
    ldr r0, =411
1:  subs r0, r0, #1
    bne 1b
    nop
    nop
    nop
    movs r3, #68
    str r3, [r7, #0x10]         @ TXF0: SM0 pulls it and outs it to the pins
    movs r2, #0
2:  adds r2, r2, #1
    ldr r0, [r7, #0x3c]         @ PINS (pin outputs)
    cmp r0, r3
    bne 2b
    str r2, [r6]                @ polls until the pins changed
    ldr r0, =448
1:  subs r0, r0, #1
    bne 1b
    nop
    nop
    nop
    nop
    movs r3, #69
    str r3, [r7, #0x10]         @ TXF0: SM0 pulls it and outs it to the pins
    movs r2, #0
2:  adds r2, r2, #1
    ldr r0, [r7, #0x3c]         @ PINS (pin outputs)
    cmp r0, r3
    bne 2b
    str r2, [r6]                @ polls until the pins changed
    ldr r0, =485
1:  subs r0, r0, #1
    bne 1b
    nop
    nop
    nop
    nop
    nop
    movs r3, #70
    str r3, [r7, #0x10]         @ TXF0: SM0 pulls it and outs it to the pins
    movs r2, #0
2:  adds r2, r2, #1
    ldr r0, [r7, #0x3c]         @ PINS (pin outputs)
    cmp r0, r3
    bne 2b
    str r2, [r6]                @ polls until the pins changed
    ldr r0, =522
1:  subs r0, r0, #1
    bne 1b
    movs r3, #71
    str r3, [r7, #0x10]         @ TXF0: SM0 pulls it and outs it to the pins
    movs r2, #0
2:  adds r2, r2, #1
    ldr r0, [r7, #0x3c]         @ PINS (pin outputs)
    cmp r0, r3
    bne 2b
    str r2, [r6]                @ polls until the pins changed
    ldr r0, =559
1:  subs r0, r0, #1
    bne 1b
    nop
    movs r3, #72
    str r3, [r7, #0x10]         @ TXF0: SM0 pulls it and outs it to the pins
    movs r2, #0
2:  adds r2, r2, #1
    ldr r0, [r7, #0x3c]         @ PINS (pin outputs)
    cmp r0, r3
    bne 2b
    str r2, [r6]                @ polls until the pins changed
    ldr r0, =0x38000
    str r0, [r5, #0x00]         @ SM0_CLKDIV = 3.5
    ldr r0, =311
1:  subs r0, r0, #1
    bne 1b
    movs r3, #65
    str r3, [r7, #0x10]         @ TXF0: SM0 pulls it and outs it to the pins
    movs r2, #0
2:  adds r2, r2, #1
    ldr r0, [r7, #0x3c]         @ PINS (pin outputs)
    cmp r0, r3
    bne 2b
    str r2, [r6]                @ polls until the pins changed
    ldr r0, =348
1:  subs r0, r0, #1
    bne 1b
    nop
    movs r3, #66
    str r3, [r7, #0x10]         @ TXF0: SM0 pulls it and outs it to the pins
    movs r2, #0
2:  adds r2, r2, #1
    ldr r0, [r7, #0x3c]         @ PINS (pin outputs)
    cmp r0, r3
    bne 2b
    str r2, [r6]                @ polls until the pins changed
    ldr r0, =385
1:  subs r0, r0, #1
    bne 1b
    nop
    nop
    movs r3, #67
    str r3, [r7, #0x10]         @ TXF0: SM0 pulls it and outs it to the pins
    movs r2, #0
2:  adds r2, r2, #1
    ldr r0, [r7, #0x3c]         @ PINS (pin outputs)
    cmp r0, r3
    bne 2b
    str r2, [r6]                @ polls until the pins changed
    ldr r0, =422
1:  subs r0, r0, #1
    bne 1b
    nop
    nop
    nop
    movs r3, #68
    str r3, [r7, #0x10]         @ TXF0: SM0 pulls it and outs it to the pins
    movs r2, #0
2:  adds r2, r2, #1
    ldr r0, [r7, #0x3c]         @ PINS (pin outputs)
    cmp r0, r3
    bne 2b
    str r2, [r6]                @ polls until the pins changed
    ldr r0, =459
1:  subs r0, r0, #1
    bne 1b
    nop
    nop
    nop
    nop
    movs r3, #69
    str r3, [r7, #0x10]         @ TXF0: SM0 pulls it and outs it to the pins
    movs r2, #0
2:  adds r2, r2, #1
    ldr r0, [r7, #0x3c]         @ PINS (pin outputs)
    cmp r0, r3
    bne 2b
    str r2, [r6]                @ polls until the pins changed
    ldr r0, =496
1:  subs r0, r0, #1
    bne 1b
    nop
    nop
    nop
    nop
    nop
    movs r3, #70
    str r3, [r7, #0x10]         @ TXF0: SM0 pulls it and outs it to the pins
    movs r2, #0
2:  adds r2, r2, #1
    ldr r0, [r7, #0x3c]         @ PINS (pin outputs)
    cmp r0, r3
    bne 2b
    str r2, [r6]                @ polls until the pins changed
    ldr r0, =533
1:  subs r0, r0, #1
    bne 1b
    movs r3, #71
    str r3, [r7, #0x10]         @ TXF0: SM0 pulls it and outs it to the pins
    movs r2, #0
2:  adds r2, r2, #1
    ldr r0, [r7, #0x3c]         @ PINS (pin outputs)
    cmp r0, r3
    bne 2b
    str r2, [r6]                @ polls until the pins changed
    ldr r0, =570
1:  subs r0, r0, #1
    bne 1b
    nop
    movs r3, #72
    str r3, [r7, #0x10]         @ TXF0: SM0 pulls it and outs it to the pins
    movs r2, #0
2:  adds r2, r2, #1
    ldr r0, [r7, #0x3c]         @ PINS (pin outputs)
    cmp r0, r3
    bne 2b
    str r2, [r6]                @ polls until the pins changed
    ldr r0, =0x874000
    str r0, [r5, #0x00]         @ SM0_CLKDIV = 135.25
    ldr r0, =311
1:  subs r0, r0, #1
    bne 1b
    movs r3, #65
    str r3, [r7, #0x10]         @ TXF0: SM0 pulls it and outs it to the pins
    movs r2, #0
2:  adds r2, r2, #1
    ldr r0, [r7, #0x3c]         @ PINS (pin outputs)
    cmp r0, r3
    bne 2b
    str r2, [r6]                @ polls until the pins changed
    ldr r0, =348
1:  subs r0, r0, #1
    bne 1b
    nop
    movs r3, #66
    str r3, [r7, #0x10]         @ TXF0: SM0 pulls it and outs it to the pins
    movs r2, #0
2:  adds r2, r2, #1
    ldr r0, [r7, #0x3c]         @ PINS (pin outputs)
    cmp r0, r3
    bne 2b
    str r2, [r6]                @ polls until the pins changed
    ldr r0, =385
1:  subs r0, r0, #1
    bne 1b
    nop
    nop
    movs r3, #67
    str r3, [r7, #0x10]         @ TXF0: SM0 pulls it and outs it to the pins
    movs r2, #0
2:  adds r2, r2, #1
    ldr r0, [r7, #0x3c]         @ PINS (pin outputs)
    cmp r0, r3
    bne 2b
    str r2, [r6]                @ polls until the pins changed
    ldr r0, =422
1:  subs r0, r0, #1
    bne 1b
    nop
    nop
    nop
    movs r3, #68
    str r3, [r7, #0x10]         @ TXF0: SM0 pulls it and outs it to the pins
    movs r2, #0
2:  adds r2, r2, #1
    ldr r0, [r7, #0x3c]         @ PINS (pin outputs)
    cmp r0, r3
    bne 2b
    str r2, [r6]                @ polls until the pins changed
    ldr r0, =459
1:  subs r0, r0, #1
    bne 1b
    nop
    nop
    nop
    nop
    movs r3, #69
    str r3, [r7, #0x10]         @ TXF0: SM0 pulls it and outs it to the pins
    movs r2, #0
2:  adds r2, r2, #1
    ldr r0, [r7, #0x3c]         @ PINS (pin outputs)
    cmp r0, r3
    bne 2b
    str r2, [r6]                @ polls until the pins changed
    ldr r0, =496
1:  subs r0, r0, #1
    bne 1b
    nop
    nop
    nop
    nop
    nop
    movs r3, #70
    str r3, [r7, #0x10]         @ TXF0: SM0 pulls it and outs it to the pins
    movs r2, #0
2:  adds r2, r2, #1
    ldr r0, [r7, #0x3c]         @ PINS (pin outputs)
    cmp r0, r3
    bne 2b
    str r2, [r6]                @ polls until the pins changed
    ldr r0, =533
1:  subs r0, r0, #1
    bne 1b
    movs r3, #71
    str r3, [r7, #0x10]         @ TXF0: SM0 pulls it and outs it to the pins
    movs r2, #0
2:  adds r2, r2, #1
    ldr r0, [r7, #0x3c]         @ PINS (pin outputs)
    cmp r0, r3
    bne 2b
    str r2, [r6]                @ polls until the pins changed
    ldr r0, =570
1:  subs r0, r0, #1
    bne 1b
    nop
    movs r3, #72
    str r3, [r7, #0x10]         @ TXF0: SM0 pulls it and outs it to the pins
    movs r2, #0
2:  adds r2, r2, #1
    ldr r0, [r7, #0x3c]         @ PINS (pin outputs)
    cmp r0, r3
    bne 2b
    str r2, [r6]                @ polls until the pins changed
park:
    b park
    .ltorg
