    .section .text.entry, "ax"
    .align 4
i2c0_base:        .word 0x60013000
cmd_stop:         .word 0x00001000
ctr_trans_start:  .word 0x00000020

    .align 4
    .global _start
_start:
    /* I2C0_BASE = 0x6001_3000.
     * Set CMD0 = STOP (opcode 2 → word 0x1000) and trigger TRANS_START
     * so the controller raises TRANS_COMPLETE (INT_RAW bit 7 = 0x80).
     * STOP, not END, because END pauses the controller and only sets
     * END_DETECT (bit 3); STOP is what closes the transaction.
     *
     * Per ESP32-S3 TRM § 29.5: opcode field is bits[13:11], so STOP=2
     * encodes as 2<<11 = 0x1000.
     */
    l32r    a2, i2c0_base           /* a2 = 0x6001_3000              */
    l32r    a3, cmd_stop            /* a3 = 0x0000_1000 (STOP)       */
    s32i    a3, a2, 0x58            /* CMD0 = STOP                   */
    l32r    a3, ctr_trans_start     /* a3 = 0x0000_0020              */
    s32i    a3, a2, 0x04            /* CTR = TRANS_START             */
    /* Read INT_RAW so the captured-mem step sees a stable value.    */
    l32i    a4, a2, 0x20
    break   1, 15
