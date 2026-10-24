/* Plain GPIO on bank-1 pads (GPIO32 and up) across two gpio nets, register
 * level: a classic ESP32 (--defsym CHIP_ESP32=1, the leader) and an
 * ESP32-S3 (the follower) hand a level back and forth. No GPIO matrix
 * signal: the output pad is FUNCn_OUT_SEL = 256 (SIG_GPIO_OUT, the OUT1
 * latch) with its ENABLE1 bit set, the input pad is read in IN1.
 *
 *   classic  out GPIO32 (OUT1 / ENABLE1 bit 0), in GPIO39 (input-only,
 *            IN1 / STATUS1 bit 7)
 *   S3       in GPIO40 (IN1 / STATUS1 bit 8), out GPIO41 (OUT1 / ENABLE1
 *            bit 9)
 *
 * Nets: classic GPIO32 -> S3 GPIO40, S3 GPIO41 -> classic GPIO39.
 *
 * Four rounds. Leader: out high, wait for in high, out low, wait for in low.
 * Follower: wait for in high, out high, wait for in low, out low. The input
 * pad's GPIO_PINn_REG.INT_TYPE is 3 (any edge), INT_ENA clear: every edge it
 * sees latches its GPIO_STATUS1 bit, which the firmware counts and clears
 * after each wait.
 *
 * RESULT[0] rounds, RESULT[1] STATUS1 edges counted, RESULT[2] the input
 * pad's STATUS1 bit when it was first armed (0), RESULT[7] = 1 done.
 */
    .include "esp32x.inc"

.ifdef CHIP_ESP32
    .set OUT_PAD, 32
    .set IN_PAD, 39
    .set IOMUX_OUT, 0x1C            /* IO_MUX_GPIO32_REG */
    .set IOMUX_IN, 0x10             /* IO_MUX_GPIO39_REG */
.else
    .set OUT_PAD, 41
    .set IN_PAD, 40
    .set IOMUX_OUT, 4 + 4 * OUT_PAD
    .set IOMUX_IN, 4 + 4 * IN_PAD
.endif
    .set OUT_BIT, 1 << (OUT_PAD - 32)
    .set IN_BIT, 1 << (IN_PAD - 32)

/* Wait until IN1's input bit reads `level`, then count and clear the edge
 * its STATUS1 bit latched. */
.macro WAIT_IN level
.Lwait_\@:
    RD a6, GPIO_IN1
    LI a3, IN_BIT
    and a6, a6, a3
.if \level
    beqz a6, .Lwait_\@
.else
    bnez a6, .Lwait_\@
.endif
    RD a6, GPIO_STATUS1
    LI a3, IN_BIT
    and a6, a6, a3
    beqz a6, .Lno_edge_\@
    INC 1
    WR GPIO_STATUS1_W1TC, IN_BIT
.Lno_edge_\@:
.endm

    .section .text,"ax"
    .global _start
_start:
    LI a14, RESULT
    /* Both pads digital: on the classic part GPIO32 (X32P) and GPIO39
     * (SENSE4) have an RTC function, so clear their RTC_IO MUX_SEL first,
     * as gpio_config() does (rtc_gpio_deinit). The S3's GPIO40/41 have
     * none. */
    RTC_DIGITAL OUT_PAD
    RTC_DIGITAL IN_PAD
    WR IO_MUX_BASE + IOMUX_OUT, PAD_MATRIX
    WR IO_MUX_BASE + IOMUX_IN, PAD_MATRIX
    /* Out: the OUT1 latch, low, driving. In: output stage off. */
    WR GPIO_BASE + GPIO_OUT_SEL0 + 4 * OUT_PAD, 256
    WR GPIO_OUT1_W1TC, OUT_BIT
    OE_ON OUT_PAD
    OE_OFF IN_PAD
    /* Any edge on the input pad latches STATUS1; start from a clear bit. */
    WR GPIO_BASE + GPIO_PIN0 + 4 * IN_PAD, 3 << 7
    WR GPIO_STATUS1_W1TC, IN_BIT
    RD a6, GPIO_STATUS1
    LI a3, IN_BIT
    and a6, a6, a3
    s32i a6, a14, 8
    movi a12, 0
.ifdef CHIP_ESP32
    /* Leader: let the follower arm its input first. */
    DELAY_US 50
.endif
.Lround:
.ifdef CHIP_ESP32
    WR GPIO_OUT1_W1TS, OUT_BIT
    WAIT_IN 1
    WR GPIO_OUT1_W1TC, OUT_BIT
    WAIT_IN 0
.else
    WAIT_IN 1
    WR GPIO_OUT1_W1TS, OUT_BIT
    WAIT_IN 0
    WR GPIO_OUT1_W1TC, OUT_BIT
.endif
    addi a12, a12, 1
    s32i a12, a14, 0
    movi a5, 4                  /* ROUNDS */
    beq a12, a5, .Ldone
    j .Lround
.Ldone:
    movi a5, 1
    s32i a5, a14, 28
.Lhalt:
    j .Lhalt
