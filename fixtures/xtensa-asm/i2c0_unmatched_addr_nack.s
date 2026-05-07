    .section .text.entry, "ax"
    .align 4
i2c0_base:        .word 0x60013000
cmd_rstart:       .word 0x00003000
cmd_write_1:      .word 0x00000801
cmd_stop:         .word 0x00001000
ctr_trans_start:  .word 0x00000020

    .align 4
    .global _start
_start:
    /* I2C0 with no slave attached at address 0x50 (`0xA0 >> 1`):
     * the controller should set INT_RAW.NACK (bit 10 = 0x400) when
     * the master writes the address byte to the bus and gets no ACK.
     *
     * Sequence:
     *   CMD0 = RSTART  (opcode 6 → 6<<11 = 0x3000)
     *   CMD1 = WRITE 1 byte (opcode 1, byte_num=1 → 0x0801)
     *   CMD2 = STOP    (opcode 2 → 2<<11 = 0x1000)
     *   DATA ← 0xA0 (addr 0x50 << 1 | W); no slave at this addr
     *   CTR = TRANS_START (bit 5 = 0x20)
     */
    l32r    a2, i2c0_base

    /* CMD0 = RSTART */
    l32r    a3, cmd_rstart
    s32i    a3, a2, 0x58

    /* CMD1 = WRITE 1 byte */
    l32r    a3, cmd_write_1
    s32i    a3, a2, 0x5C

    /* CMD2 = STOP */
    l32r    a3, cmd_stop
    s32i    a3, a2, 0x60

    /* DATA register at offset 0x1c: push the unmatched address byte. */
    movi    a3, 0xA0
    s32i    a3, a2, 0x1C

    /* CTR = TRANS_START */
    l32r    a3, ctr_trans_start
    s32i    a3, a2, 0x04

    /* Capture INT_RAW into a4 — caller verifies bit 10 (NACK) is set. */
    l32i    a4, a2, 0x20
    break   1, 15
