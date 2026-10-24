/* RTC GPIO across gpio nets, register level: a classic ESP32
 * (--defsym CHIP_ESP32=1, the leader) and an ESP32-S3 (the follower) hand a
 * level back and forth through pads the RTC IO MUX owns (RTC_IO MUX_SEL = 1),
 * then the leader hands a pad from the RTC domain back to the digital one.
 *
 *   classic  GPIO25 = RTC_GPIO6 (PAD_DAC1, bit 14 + 6 = 20): open-drain out
 *            (RTC_GPIO_PIN6.PAD_DRIVER)
 *            GPIO26 = RTC_GPIO7 (PAD_DAC2, bit 21): in, FUN_IE
 *            GPIO27 = RTC_GPIO17 (TOUCH_PAD7): the hand-off pad
 *   S3       GPIO4 = RTC_GPIO4 (TOUCH_PAD4, bit 10 + 4 = 14): in, FUN_IE,
 *            RUE (the pull-up the leader's open drain releases to)
 *            GPIO5 = RTC_GPIO5 (TOUCH_PAD5, bit 15): push-pull out
 *            GPIO6 = RTC_GPIO6 (TOUCH_PAD6, bit 16): in, FUN_IE, RDE
 *
 * Nets (no board pull on any of them): a2b classic GPIO25 -> S3 GPIO4,
 * b2a S3 GPIO5 -> classic GPIO26, handoff classic GPIO27 -> S3 GPIO6.
 *
 * Four rounds. Leader: release (the S3's RUE pulls the net high), wait for
 * the answer high, drive low, wait for it low. Follower: wait for high,
 * answer high, wait for low, answer low. Each input pad's RTC_GPIO_PINn.
 * INT_TYPE is 3 (any edge): every edge it sees latches its RTC_GPIO_STATUS
 * bit, which the firmware counts and clears (RTC_GPIO_STATUS_W1TC) after each
 * wait. The RTC GPIO latch reaches no CPU interrupt on either part (see
 * crates/core/src/peripherals/esp_rtc_io.rs), so it is polled.
 *
 * Then the hand-off. The leader has GPIO27 set up as a plain digital output
 * driving 1 (FUNC27_OUT_SEL = 256, OUT bit 27, ENABLE bit 27) from the start,
 * but with TOUCH_PAD7.MUX_SEL set and the RTC output stage off, so the pad
 * does NOT carry it: the S3's RDE holds the net low. A fifth round tells the
 * S3 to sample its GPIO6 (0); the leader then clears MUX_SEL, the pad carries
 * the digital 1, and the round's second half tells the S3 to sample again
 * (1). The S3's GPIO6 has INT_TYPE 1 (rising): that edge latches its STATUS
 * bit.
 *
 * RESULT[0] rounds, RESULT[1] STATUS edges counted, RESULT[2] the input
 * pad's STATUS bit when first armed (0), RESULT[7] = 1 done.
 * Leader: RESULT[3] GPIO_IN bit 27 after the hand-off (1, the pad reads back
 * the level it drives), RESULT[4] GPIO_IN bit 27 before it (0: in the RTC
 * domain the pad's input buffer follows TOUCH_PAD7.FUN_IE, which is clear).
 * Follower: RESULT[3] / RESULT[4] RTC_GPIO_IN bit 16 before / after the
 * hand-off (0 / 1), RESULT[5] its STATUS bit 16 (the rising edge, 1).
 */
    .include "esp32x.inc"

.ifdef CHIP_ESP32
    /* PAD_DACn: FUN_IE 11, MUX_SEL 17; TOUCH_PADn: FUN_IE 13, MUX_SEL 19. */
    .set PAD_OUT, RTCIO_BASE + 0x84         /* PAD_DAC1, GPIO25 */
    .set PAD_IN, RTCIO_BASE + 0x88          /* PAD_DAC2, GPIO26 */
    .set PAD_HANDOFF, RTCIO_BASE + 0xB0     /* TOUCH_PAD7, GPIO27 */
    .set OUT_RTC, 6
    .set IN_RTC, 7
    .set OUT_CFG, 1 << 17
    .set IN_CFG, (1 << 17) | (1 << 11)
    .set HANDOFF_PAD, 27
    .set HANDOFF_MUX, 1 << 19
    .set IOMUX_HANDOFF, 0x24                /* IO_MUX_GPIO27_REG */
.else
    /* TOUCH_PADn: FUN_IE 13, MUX_SEL 19, RUE 27, RDE 28. */
    .set PAD_IN, RTCIO_BASE + 0x84 + 4 * 4  /* TOUCH_PAD4, GPIO4 */
    .set PAD_OUT, RTCIO_BASE + 0x84 + 4 * 5 /* TOUCH_PAD5, GPIO5 */
    .set PAD_HANDOFF, RTCIO_BASE + 0x84 + 4 * 6 /* TOUCH_PAD6, GPIO6 */
    .set IN_RTC, 4
    .set OUT_RTC, 5
    .set HANDOFF_RTC, 6
    .set PULLS, (1 << 27) | (1 << 28)
    .set IN_CFG, (1 << 19) | (1 << 13) | (1 << 27)
    .set OUT_CFG, 1 << 19
    .set HANDOFF_CFG, (1 << 19) | (1 << 13) | (1 << 28)
    .set HANDOFF_BIT, 1 << (RTC_SHIFT + HANDOFF_RTC)
.endif
    .set OUT_BIT, 1 << (RTC_SHIFT + OUT_RTC)
    .set IN_BIT, 1 << (RTC_SHIFT + IN_RTC)

/* *reg = (*reg & ~clr) | set, read-modify-write (uses a2..a4). */
.macro RMW addr, clr, set
    RD a4, \addr
    LI a3, ~(\clr)
    and a4, a4, a3
    LI a3, \set
    or a4, a4, a3
    WRR \addr, a4
.endm

/* Wait until RTC_GPIO_IN's input bit reads `level`, then count and clear
 * the edge its RTC_GPIO_STATUS bit latched. */
.macro WAIT_IN level
.Lwait_\@:
    RD a6, RTC_GPIO_IN
    LI a3, IN_BIT
    and a6, a6, a3
.if \level
    beqz a6, .Lwait_\@
.else
    bnez a6, .Lwait_\@
.endif
    RD a6, RTC_GPIO_STATUS
    LI a3, IN_BIT
    and a6, a6, a3
    beqz a6, .Lno_edge_\@
    INC 1
    WR RTC_GPIO_STATUS_W1TC, IN_BIT
.Lno_edge_\@:
.endm

/* *(u32 *)(RESULT + 4 * idx) = (*addr & mask) != 0 (uses a2, a3, a5, a6;
 * llvm-mc takes no symbol in EXTUI's shift field). */
.macro STORE_BIT idx, addr, mask
    RD a6, \addr
    LI a3, \mask
    and a6, a6, a3
    movi a5, 1
    movnez a6, a5, a6
    s32i a6, a14, 4 * \idx
.endm

/* One handshake round, leader side: release, wait high, drive low, wait
 * low. The follower side answers. */
.macro ROUND_LEADER
    WR RTC_GPIO_OUT_W1TS, OUT_BIT
    WAIT_IN 1
    WR RTC_GPIO_OUT_W1TC, OUT_BIT
    WAIT_IN 0
.endm

    .section .text,"ax"
    .global _start
_start:
    LI a14, RESULT
.ifdef CHIP_ESP32
    /* Out: RTC_GPIO6 open drain, holding 0, on (esp-idf rtc_gpio_init +
     * rtc_gpio_set_direction(RTC_GPIO_MODE_OUTPUT_OD)). */
    RMW PAD_OUT, 0, OUT_CFG
    WR RTC_GPIO_PIN0 + 4 * OUT_RTC, PAD_DRIVER
    WR RTC_GPIO_OUT_W1TC, OUT_BIT
    WR RTC_GPIO_ENABLE_W1TS, OUT_BIT
    /* In: RTC_GPIO7, input buffer on. */
    RMW PAD_IN, 0, IN_CFG
    /* The hand-off pad: a plain digital output driving 1 through the matrix
     * bypass, but owned by the RTC IO MUX (its RTC output stage off). */
    RMW PAD_HANDOFF, 0, HANDOFF_MUX
    WR IO_MUX_BASE + IOMUX_HANDOFF, PAD_MATRIX
    WR GPIO_BASE + GPIO_OUT_SEL0 + 4 * HANDOFF_PAD, 256
    WR GPIO_OUT_W1TS, 1 << HANDOFF_PAD
    OE_ON HANDOFF_PAD
.else
    /* In: RTC_GPIO4 with its pull-up (the leader's open drain releases to
     * it); TOUCH_PAD4 resets with RDE, which goes. */
    RMW PAD_IN, PULLS, IN_CFG
    /* Out: RTC_GPIO5 push-pull, 0. */
    WR RTC_GPIO_OUT_W1TC, OUT_BIT
    WR RTC_GPIO_ENABLE_W1TS, OUT_BIT
    RMW PAD_OUT, 0, OUT_CFG
    /* The hand-off pad: RTC_GPIO6 in, pulled down, rising edges latched. */
    RMW PAD_HANDOFF, PULLS, HANDOFF_CFG
    WR RTC_GPIO_PIN0 + 4 * HANDOFF_RTC, 1 << 7
.endif
    /* Any edge on the input pad latches its STATUS bit. */
    WR RTC_GPIO_PIN0 + 4 * IN_RTC, 3 << 7
.ifdef CHIP_ESP32
    /* Leader: let the follower arm its pads first, then start from a clear
     * bit (the follower's setup drove b2a low meanwhile). */
    DELAY_US 50
    WR RTC_GPIO_STATUS_W1TC, IN_BIT
.else
    /* Follower: the leader is ready once it holds a2b low. */
.Lready:
    RD a6, RTC_GPIO_IN
    LI a3, IN_BIT
    and a6, a6, a3
    bnez a6, .Lready
    WR RTC_GPIO_STATUS_W1TC, IN_BIT | HANDOFF_BIT
.endif
    RD a6, RTC_GPIO_STATUS
    LI a3, IN_BIT
    and a6, a6, a3
    s32i a6, a14, 8
    movi a12, 0
.Lround:
.ifdef CHIP_ESP32
    ROUND_LEADER
.else
    WAIT_IN 1
    WR RTC_GPIO_OUT_W1TS, OUT_BIT
    WAIT_IN 0
    WR RTC_GPIO_OUT_W1TC, OUT_BIT
.endif
    addi a12, a12, 1
    s32i a12, a14, 0
    movi a5, 4                  /* ROUNDS */
    beq a12, a5, .Lhandoff
    j .Lround
.Lhandoff:
.ifdef CHIP_ESP32
    /* In the RTC domain GPIO27's input buffer is TOUCH_PAD7.FUN_IE (clear):
     * GPIO_IN reads 0 although the digital side drives 1. */
    STORE_BIT 4, GPIO_BASE + 0x3C, 1 << HANDOFF_PAD
    WR RTC_GPIO_OUT_W1TS, OUT_BIT
    WAIT_IN 1
    /* MUX_SEL clear: the pad is the digital output's again. */
    RMW PAD_HANDOFF, HANDOFF_MUX, 0
    WR RTC_GPIO_OUT_W1TC, OUT_BIT
    WAIT_IN 0
    STORE_BIT 3, GPIO_BASE + 0x3C, 1 << HANDOFF_PAD
.else
    WAIT_IN 1
    STORE_BIT 3, RTC_GPIO_IN, HANDOFF_BIT
    WR RTC_GPIO_OUT_W1TS, OUT_BIT
    WAIT_IN 0
    STORE_BIT 4, RTC_GPIO_IN, HANDOFF_BIT
    STORE_BIT 5, RTC_GPIO_STATUS, HANDOFF_BIT
    WR RTC_GPIO_OUT_W1TC, OUT_BIT
.endif
    movi a5, 1
    s32i a5, a14, 28
.Lhalt:
    j .Lhalt
