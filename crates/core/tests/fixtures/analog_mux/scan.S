@ Knob scan for the 74HC4051 e2e test (tests/analog_mux_e2e.rs), in the
@ shape of a real knob scanner: set the select lines, start one conversion
@ at once, wait for COCO0, store R0. The bytes are in the test; rebuild with
@   arm-none-eabi-as -mcpu=cortex-m7 -mthumb scan.S -o scan.o
@   arm-none-eabi-objcopy -O binary scan.o scan.bin
@ r0 = GPIO1 base, r1 = ADC1 base, r2 = table (u16[8]), r3 = channel.
    .syntax unified
    .thumb
    ldr   r0, =0x401B8000
    ldr   r1, =0x400C4000
    ldr   r2, =0x20001000
    movw  r4, #0x208         @ CFG.MODE = 12-bit
    str   r4, [r1, #0x44]
    movs  r4, #0x70          @ GDIR: PAD_S0..S2 (bits 4..6) are outputs
    str   r4, [r0, #4]
    movs  r3, #0
1:  lsls  r4, r3, #4         @ DR = channel << 4 (S0 = bit 4)
    str   r4, [r0, #0]
    movs  r4, #3             @ HC0 = ADC channel 3: start a conversion
    str   r4, [r1, #0]
2:  ldr   r4, [r1, #0x20]    @ HS.COCO0
    lsls  r4, r4, #31
    beq   2b
    ldr   r4, [r1, #0x24]    @ R0
    strh  r4, [r2, r3, lsl #1]
    adds  r3, r3, #1
    cmp   r3, #8
    bne   1b
3:  b     3b
    .ltorg
