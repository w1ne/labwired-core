/* ESP32-S3 and classic ESP32 (Xtensa) register level, no IDF: what the
 * gpio-net-buses Xtensa firmware needs. Assembled with --defsym CHIP_ESP32=1
 * for the classic ESP32, without it for the ESP32-S3.
 *
 * --defsym INST1=1 picks the second controller of a kind: S3 GP-SPI3 and
 * I2C1, classic I2C1 (classic SPI is VSPI master / HSPI slave either way).
 * --defsym PADS_HI=1 moves the pads to bank 1 (GPIO32 and up: OUT1 /
 * ENABLE1 / IN1 at bit pad - 32, PINn and FUNCn by the pad number itself).
 *
 * Hand-written assembly: LLVM 18 assembles Xtensa (llvm-mc -triple=xtensa,
 * the core ISA, call0 ABI) but generates no code for it and ld.lld cannot
 * link it, so there is no C here; build.sh assembles each source to a
 * one-section object and src/xtensa_elf.py places it at the IRAM entry the
 * simulator's bare-ELF fast boot jumps to. Every literal sits in the code
 * next to the l32r that loads it (LI), so the object has no relocations.
 *
 * Registers: a2..a5 are scratch for the macros below, a14 holds RESULT,
 * a0 is the call0 return address (no subroutine calls another).
 *
 * Pads (GPIO matrix, IO_MUX function GPIO):
 *   S3       SPI2  GPIO12 SCK, GPIO11 MOSI, GPIO13 MISO, GPIO10 CS0
 *            I2C0  GPIO8 SDA, GPIO9 SCL (open drain, PAD_DRIVER)
 *   classic  VSPI (master) / HSPI (slave)  GPIO18 SCK, GPIO23 MOSI,
 *            GPIO19 MISO, GPIO5 CS0
 *            I2C0  GPIO21 SDA, GPIO22 SCL (open drain, PAD_DRIVER)
 * PADS_HI:
 *   S3       SPI   GPIO36 SCK, GPIO35 MOSI, GPIO37 MISO, GPIO34 CS0
 *            I2C   GPIO41 SDA, GPIO42 SCL
 *   classic  GPIO34..39 are input-only, so the pads depend on the role:
 *            VSPI master  GPIO33 SCK, GPIO32 MOSI, GPIO34 MISO, GPIO5 CS0
 *            HSPI slave   GPIO34 SCK, GPIO35 MOSI, GPIO33 MISO, GPIO36 CS0
 *            (a source sets SPI_ROLE_SLAVE before the include)
 *            I2C   GPIO32 SDA, GPIO33 SCL
 */

.ifdef CHIP_ESP32
/* ── classic ESP32 (esp-idf soc/esp32/include/soc/*.h, TRM v4.6) ───────── */
    .set GPIO_BASE, 0x3FF44000
    .set GPIO_PIN0, 0x88            /* GPIO_PINn_REG */
    .set GPIO_IN_SEL0, 0x130        /* GPIO_FUNCn_IN_SEL_CFG_REG */
    .set GPIO_OUT_SEL0, 0x530       /* GPIO_FUNCn_OUT_SEL_CFG_REG */
    .set IO_MUX_BASE, 0x3FF49000
    /* MCU_SEL [14:12] = 2 (GPIO), FUN_DRV [11:10] = 2, FUN_IE (9). */
    .set PAD_MATRIX, (2 << 12) | (2 << 10) | (1 << 9)
    .set RESULT, 0x3FFC0100
.ifdef INST1
    .set I2C0, 0x3FF67000           /* I2C1 (I2C_EXT1) */
.else
    .set I2C0, 0x3FF53000
.endif
    .set SPI_MASTER, 0x3FF65000     /* VSPI (SPI3) */
    .set SPI_SLAVE, 0x3FF64000      /* HSPI (SPI2) */
    /* gpio_sig_map.h */
    .set SIG_M_CLK, 63              /* VSPICLK */
    .set SIG_M_Q, 64                /* VSPIQ */
    .set SIG_M_D, 65                /* VSPID */
    .set SIG_M_CS0, 68              /* VSPICS0 */
    .set SIG_S_CLK, 8               /* HSPICLK */
    .set SIG_S_Q, 9                 /* HSPIQ */
    .set SIG_S_D, 10                /* HSPID */
    .set SIG_S_CS0, 11              /* HSPICS0 */
.ifdef INST1
    .set SIG_I2C_SCL, 95            /* I2CEXT1_SCL */
    .set SIG_I2C_SDA, 96            /* I2CEXT1_SDA */
.else
    .set SIG_I2C_SCL, 29
    .set SIG_I2C_SDA, 30
.endif
.ifdef PADS_HI
  .ifdef SPI_ROLE_SLAVE
    .set PAD_SCK, 34
    .set PAD_MOSI, 35
    .set PAD_MISO, 33
    .set PAD_CS, 36
    .set IOMUX_SCK, 0x14
    .set IOMUX_MOSI, 0x18
    .set IOMUX_MISO, 0x20
    .set IOMUX_CS, 0x04
  .else
    .set PAD_SCK, 33
    .set PAD_MOSI, 32
    .set PAD_MISO, 34
    .set PAD_CS, 5
    .set IOMUX_SCK, 0x20
    .set IOMUX_MOSI, 0x1C
    .set IOMUX_MISO, 0x14
    .set IOMUX_CS, 0x6C
  .endif
    .set PAD_SDA, 32
    .set PAD_SCL, 33
    .set IOMUX_SDA, 0x1C
    .set IOMUX_SCL, 0x20
.else
    .set PAD_SCK, 18
    .set PAD_MOSI, 23
    .set PAD_MISO, 19
    .set PAD_CS, 5
    .set PAD_SDA, 21
    .set PAD_SCL, 22
    /* IO_MUX_GPIOn_REG offsets (io_mux_reg.h: not in pad order) */
    .set IOMUX_SCK, 0x70
    .set IOMUX_MOSI, 0x8C
    .set IOMUX_MISO, 0x74
    .set IOMUX_CS, 0x6C
    .set IOMUX_SDA, 0x7C
    .set IOMUX_SCL, 0x80
.endif
.else
/* ── ESP32-S3 (esp-idf soc/esp32s3/include/soc/*.h, TRM) ───────────────── */
    .set GPIO_BASE, 0x60004000
    .set GPIO_PIN0, 0x74
    .set GPIO_IN_SEL0, 0x154
    .set GPIO_OUT_SEL0, 0x554
    .set IO_MUX_BASE, 0x60009000
    /* MCU_SEL [14:12] = 1 (GPIO), FUN_DRV [11:10] = 2, FUN_IE (9). */
    .set PAD_MATRIX, (1 << 12) | (2 << 10) | (1 << 9)
    .set RESULT, 0x3FCE0100
.ifdef INST1
    .set I2C0, 0x60027000           /* I2C1 */
    .set SPI_MASTER, 0x60025000     /* GP-SPI3, both roles */
    .set SPI_SLAVE, 0x60025000
    /* gpio_sig_map.h: SPI3_CLK 66, SPI3_Q 67, SPI3_D 68, SPI3_CS0 71 */
    .set SIG_M_CLK, 66
    .set SIG_M_Q, 67
    .set SIG_M_D, 68
    .set SIG_M_CS0, 71
    .set SIG_I2C_SCL, 91            /* I2CEXT1_SCL */
    .set SIG_I2C_SDA, 92            /* I2CEXT1_SDA */
.else
    .set I2C0, 0x60013000
    .set SPI_MASTER, 0x60024000     /* GP-SPI2 (FSPI), both roles */
    .set SPI_SLAVE, 0x60024000
    .set SIG_M_CLK, 101             /* FSPICLK */
    .set SIG_M_Q, 102               /* FSPIQ */
    .set SIG_M_D, 103               /* FSPID */
    .set SIG_M_CS0, 110             /* FSPICS0 */
    .set SIG_I2C_SCL, 89
    .set SIG_I2C_SDA, 90
.endif
    .set SIG_S_CLK, SIG_M_CLK
    .set SIG_S_Q, SIG_M_Q
    .set SIG_S_D, SIG_M_D
    .set SIG_S_CS0, SIG_M_CS0
.ifdef PADS_HI
    .set PAD_SCK, 36
    .set PAD_MOSI, 35
    .set PAD_MISO, 37
    .set PAD_CS, 34
    .set PAD_SDA, 41
    .set PAD_SCL, 42
.else
    .set PAD_SCK, 12
    .set PAD_MOSI, 11
    .set PAD_MISO, 13
    .set PAD_CS, 10
    .set PAD_SDA, 8
    .set PAD_SCL, 9
.endif
    .set IOMUX_SCK, 4 + 4 * PAD_SCK
    .set IOMUX_MOSI, 4 + 4 * PAD_MOSI
    .set IOMUX_MISO, 4 + 4 * PAD_MISO
    .set IOMUX_CS, 4 + 4 * PAD_CS
    .set IOMUX_SDA, 4 + 4 * PAD_SDA
    .set IOMUX_SCL, 4 + 4 * PAD_SCL
.endif

/* RTC_IO (RTC IO MUX + RTC GPIO): esp-idf soc/<chip>/include/soc/
 * rtc_io_reg.h, the vendored SVDs. RTC GPIO n is bit RTC_SHIFT + n of
 * RTC_GPIO_OUT / ENABLE / STATUS / IN: 14 + n on the classic part, 10 + n on
 * the S3. Every pad register's MUX_SEL resets to 0 (digital). */
.ifdef CHIP_ESP32
    .set RTCIO_BASE, 0x3FF48400
    .set RTC_SHIFT, 14
.else
    .set RTCIO_BASE, 0x60008400
    .set RTC_SHIFT, 10
.endif
    .set RTC_GPIO_OUT_W1TS, RTCIO_BASE + 0x04
    .set RTC_GPIO_OUT_W1TC, RTCIO_BASE + 0x08
    .set RTC_GPIO_ENABLE_W1TS, RTCIO_BASE + 0x10
    .set RTC_GPIO_STATUS, RTCIO_BASE + 0x18
    .set RTC_GPIO_STATUS_W1TC, RTCIO_BASE + 0x20
    .set RTC_GPIO_IN, RTCIO_BASE + 0x24
    .set RTC_GPIO_PIN0, RTCIO_BASE + 0x28

/* The pad register and MUX_SEL bit of GPIO `pad`, as RTC_PAD_REG /
 * RTC_PAD_MUX; RTC_PAD_REG = 0 for a pad with no RTC function. Classic:
 * esp-idf soc/esp32/rtc_io_periph.c (SENSOR_PADS 0x7C for GPIO36..39,
 * ADC_PAD 0x80 for GPIO34/35, PAD_DAC1/2 0x84/0x88 for GPIO25/26,
 * XTAL_32K_PAD 0x8C for GPIO32/33, TOUCH_PAD0..7 0x94.. for GPIO4, 0, 2,
 * 15, 13, 12, 14, 27). S3: GPIO0..21, TOUCH_PAD0..14 at 0x84 + 4n,
 * XTAL_32P/32N, PAD_DAC1/2, RTC_PAD19..21 from 0xC0 on, MUX_SEL bit 19. */
.macro RTC_PAD pad
    .set RTC_PAD_REG, 0
    .set RTC_PAD_MUX, 19
.ifdef CHIP_ESP32
  .if \pad >= 36 && \pad <= 39
    .set RTC_PAD_REG, 0x7C
    .set RTC_PAD_MUX, 27 - (\pad - 36)
  .elseif \pad == 34
    .set RTC_PAD_REG, 0x80
    .set RTC_PAD_MUX, 29
  .elseif \pad == 35
    .set RTC_PAD_REG, 0x80
    .set RTC_PAD_MUX, 28
  .elseif \pad == 25 || \pad == 26
    .set RTC_PAD_REG, 0x84 + 4 * (\pad - 25)
    .set RTC_PAD_MUX, 17
  .elseif \pad == 32
    .set RTC_PAD_REG, 0x8C
    .set RTC_PAD_MUX, 17
  .elseif \pad == 33
    .set RTC_PAD_REG, 0x8C
    .set RTC_PAD_MUX, 18
  .elseif \pad == 4
    .set RTC_PAD_REG, 0x94
  .elseif \pad == 0
    .set RTC_PAD_REG, 0x98
  .elseif \pad == 2
    .set RTC_PAD_REG, 0x9C
  .elseif \pad == 15
    .set RTC_PAD_REG, 0xA0
  .elseif \pad == 13
    .set RTC_PAD_REG, 0xA4
  .elseif \pad == 12
    .set RTC_PAD_REG, 0xA8
  .elseif \pad == 14
    .set RTC_PAD_REG, 0xAC
  .elseif \pad == 27
    .set RTC_PAD_REG, 0xB0
  .endif
.else
  .if \pad <= 14
    .set RTC_PAD_REG, 0x84 + 4 * \pad
  .elseif \pad <= 18
    .set RTC_PAD_REG, 0xC0 + 4 * (\pad - 15)
  .elseif \pad <= 21
    .set RTC_PAD_REG, 0xD0 + 4 * (\pad - 19)
  .endif
.endif
.endm

    .set GPIO_OUT_W1TS, GPIO_BASE + 0x08
    .set GPIO_OUT1_W1TS, GPIO_BASE + 0x14
    .set GPIO_OUT1_W1TC, GPIO_BASE + 0x18
    .set GPIO_ENABLE_W1TS, GPIO_BASE + 0x24
    .set GPIO_ENABLE_W1TC, GPIO_BASE + 0x28
    .set GPIO_ENABLE1_W1TS, GPIO_BASE + 0x30
    .set GPIO_ENABLE1_W1TC, GPIO_BASE + 0x34
    .set GPIO_IN1, GPIO_BASE + 0x40
    .set GPIO_STATUS1, GPIO_BASE + 0x50
    .set GPIO_STATUS1_W1TC, GPIO_BASE + 0x58
    .set PAD_DRIVER, 1 << 2
    .set PAD_WPU, 1 << 8
    /* FUNCn_IN_SEL_CFG: pad [5:0], SEL (through the matrix) bit 7 on both. */
    .set IN_SEL, 1 << 7

/* I2C0 (I2C1 with INST1): the command-list controller, same offsets on
 * both parts and both instances. */
    .set I2C_SCL_LOW, I2C0 + 0x00
    .set I2C_CTR, I2C0 + 0x04
    .set I2C_SR, I2C0 + 0x08
    .set I2C_SLAVE_ADDR, I2C0 + 0x10
    .set I2C_FIFO_CONF, I2C0 + 0x18
    .set I2C_DATA, I2C0 + 0x1C
    .set I2C_INT_RAW, I2C0 + 0x20
    .set I2C_INT_CLR, I2C0 + 0x24
    .set I2C_SDA_HOLD, I2C0 + 0x30
    .set I2C_SDA_SAMPLE, I2C0 + 0x34
    .set I2C_SCL_HIGH, I2C0 + 0x38
    .set I2C_START_HOLD, I2C0 + 0x40
    .set I2C_RSTART_SETUP, I2C0 + 0x44
    .set I2C_STOP_HOLD, I2C0 + 0x48
    .set I2C_STOP_SETUP, I2C0 + 0x4C
    .set I2C_CLK_CONF, I2C0 + 0x54      /* S3 only */
    .set I2C_COMD0, I2C0 + 0x58
    .set I2C_STRETCH_CONF, I2C0 + 0x84  /* S3 only */
    .set I2C_TRANS_START, 1 << 5
    .set I2C_MS_MODE, 1 << 4
    .set I2C_FORCE_OUT, 3
    .set I2C_CLK_EN, 1 << 8
    .set INT_RXFIFO_WM, 1 << 0
    .set INT_TRANS_COMPLETE, 1 << 7
    .set INT_NACK, 1 << 10
    .set INT_SLAVE_STRETCH, 1 << 16
    .set STRETCH_EN, 1 << 10
    .set STRETCH_CLR, 1 << 11
    .set ACK_CHECK, 1 << 8
    .set NACK_LAST, 1 << 10
.ifdef CHIP_ESP32
    /* hal/esp32/include/hal/i2c_ll.h: RSTART 0, WRITE 1, READ 2, STOP 3 */
    .set OP_RSTART, 0
    .set OP_WRITE, 1
    .set OP_READ, 2
    .set OP_STOP, 3
    .set I2C_CONF_UPGATE, 0
    /* FIFO_CONF: TX_RST 13, RX_RST 12, TXFIFO_EMPTY_THRHD 8, RXFIFO_FULL_THRHD 1 */
    .set FIFO_RUN, (8 << 5) | 1
.else
    /* esp32s3 PAC i2c0::comd: RSTART 6, WRITE 1, READ 3, STOP 2 */
    .set OP_RSTART, 6
    .set OP_WRITE, 1
    .set OP_READ, 3
    .set OP_STOP, 2
    .set I2C_CONF_UPGATE, 1 << 11
    /* FIFO_PRT_EN 14 as well */
    .set FIFO_RUN, (1 << 14) | (8 << 5) | 1
.endif
    .set FIFO_RESET, FIFO_RUN | (1 << 13) | (1 << 12)
    .set FIFO_TX_RESET, FIFO_RUN | (1 << 13)

/* ── macros ─────────────────────────────────────────────────────────────── */

/* reg = 32-bit constant, from a literal placed just before the l32r. */
.macro LI reg, val
    j .Lli_\@
    .balign 4
.Llit_\@:
    .long \val
.Lli_\@:
    l32r \reg, .Llit_\@
.endm

/* *(u32 *)addr = val */
.macro WR addr, val
    LI a2, \addr
    LI a3, \val
    s32i a3, a2, 0
    memw
.endm

/* *(u32 *)addr = reg */
.macro WRR addr, reg
    LI a2, \addr
    s32i \reg, a2, 0
    memw
.endm

/* reg = *(u32 *)addr */
.macro RD reg, addr
    LI a2, \addr
    memw
    l32i \reg, a2, 0
.endm

/* RESULT[idx] += 1 (uses a4) */
.macro INC idx
    l32i a4, a14, 4 * \idx
    addi a4, a4, 1
    s32i a4, a14, 4 * \idx
.endm

/* About `us` microseconds at 240 MHz, two instructions an iteration. */
.macro DELAY_US us
    LI a3, \us * 120
.Ldly_\@:
    addi a3, a3, -1
    bnez a3, .Ldly_\@
.endm

/* Output stage of `pad` on / off: ENABLE for GPIO0..31, ENABLE1 at bit
 * pad - 32 above. */
.macro OE_ON pad
.if \pad >= 32
    WR GPIO_ENABLE1_W1TS, 1 << (\pad - 32)
.else
    WR GPIO_ENABLE_W1TS, 1 << \pad
.endif
.endm

.macro OE_OFF pad
.if \pad >= 32
    WR GPIO_ENABLE1_W1TC, 1 << (\pad - 32)
.else
    WR GPIO_ENABLE_W1TC, 1 << \pad
.endif
.endm

/* Hand an RTC-capable `pad` to the digital IO_MUX: clear its RTC_IO
 * MUX_SEL, read-modify-write, as ESP-IDF's gpio_config() does through
 * rtc_gpio_deinit() before using a pad digitally (and esp-hal's pin init).
 * Nothing for a pad with no RTC function. Uses a3, a4. */
.macro RTC_DIGITAL pad
    RTC_PAD \pad
.if RTC_PAD_REG
    RD a4, RTCIO_BASE + RTC_PAD_REG
    LI a3, ~(1 << RTC_PAD_MUX)
    and a4, a4, a3
    WRR RTCIO_BASE + RTC_PAD_REG, a4
.endif
.endm

/* Route `pad` (IO_MUX word at `iomux`) as a matrix output of `sig`. FUNCn
 * takes the pad number itself, in either bank. */
.macro MATRIX_OUT pad, iomux, sig, padbits
    RTC_DIGITAL \pad
    WR IO_MUX_BASE + \iomux, PAD_MATRIX | \padbits
    WR GPIO_BASE + GPIO_OUT_SEL0 + 4 * \pad, \sig
    OE_ON \pad
.endm

/* Route input `sig` from `pad` (output stage off). */
.macro MATRIX_IN pad, iomux, sig
    RTC_DIGITAL \pad
    WR IO_MUX_BASE + \iomux, PAD_MATRIX
    OE_OFF \pad
    WR GPIO_BASE + GPIO_IN_SEL0 + 4 * \sig, IN_SEL | \pad
.endm

/* OUT latch of the two pads `a` and `b` (one bank) to 1. */
.macro OUT_HIGH2 a, b
.if \a >= 32
    WR GPIO_OUT1_W1TS, (1 << (\a - 32)) | (1 << (\b - 32))
.else
    WR GPIO_OUT_W1TS, (1 << \a) | (1 << \b)
.endif
.endm

/* Peripheral clocks on, out of reset. S3: SYSTEM_PERIP_CLK_EN0 /
 * PERIP_RST_EN0 bit 6 SPI2, bit 7 I2C_EXT0 (INST1: bit 16 SPI3, bit 18
 * I2C_EXT1). Classic: DPORT_PERIP_CLK_EN / PERIP_RST_EN bit 6 HSPI, bit 7
 * I2C_EXT0, bit 16 VSPI (INST1: bit 18 I2C_EXT1). */
.ifdef INST1
    .set CLK_BITS_S3, (1 << 16) | (1 << 18)
    .set CLK_BITS_ESP32, (1 << 6) | (1 << 16) | (1 << 18)
.else
    .set CLK_BITS_S3, (1 << 6) | (1 << 7)
    .set CLK_BITS_ESP32, (1 << 6) | (1 << 7) | (1 << 16)
.endif
.macro ENABLE_CLOCKS
.ifdef CHIP_ESP32
    WR 0x3FF000C0, CLK_BITS_ESP32
    WR 0x3FF000C4, 0
.else
    RD a4, 0x600C0018
    LI a3, CLK_BITS_S3
    or a4, a4, a3
    WRR 0x600C0018, a4
    WR 0x600C0020, 0
.endif
.endm
