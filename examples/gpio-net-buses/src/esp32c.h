/* ESP32-C3 / ESP32-C6 register level, no IDF: what the gpio-net-buses ESP
 * firmware needs. Build with -DCHIP_C6 for the ESP32-C6 (same GP-SPI and I2C
 * register files at other bases, its own GPIO-matrix numbers).
 *
 * Pads (GPIO matrix only; none of them has an IO_MUX function for these
 * signals, so esp-idf and esp-hal route them through the matrix as well):
 *   SPI2  GPIO4 SCK, GPIO5 MOSI, GPIO3 MISO, GPIO1 CS0
 *   I2C0  GPIO8 SDA, GPIO9 SCL (open drain, PAD_DRIVER)
 */
#ifndef ESP32C_H
#define ESP32C_H
#include <stdint.h>

#define REG(a) (*(volatile uint32_t *)(a))

#ifdef CHIP_C6
#define GPIO_BASE   0x60091000u
#define IO_MUX_BASE 0x60090000u
#define SPI2_BASE   0x60081000u
#define I2C0_BASE   0x60004000u
#define GDMA_BASE   0x60080000u
#define GDMA_OUT_CONF0_CH0 0x190u
#define SRAM_BASE   0x40800000u
#define RESULT ((volatile uint32_t *)0x40800100u)
/* esp32c6 gpio_sig_map / PAC: FUNCn_IN_SEL_CFG pad [5:0], SEL bit 7. */
#define SIG_I2C_SCL 45u
#define SIG_I2C_SDA 46u
#define IN_SEL(pad) ((1u << 7) | (pad))
#else
#define GPIO_BASE   0x60004000u
#define IO_MUX_BASE 0x60009000u
#define SPI2_BASE   0x60024000u
#define I2C0_BASE   0x60013000u
#define GDMA_BASE   0x6003F000u
#define GDMA_OUT_CONF0_CH0 0x0D0u
#define SRAM_BASE   0x3FC80000u
#define RESULT ((volatile uint32_t *)0x3FC80100u)
/* esp32c3 gpio_sig_map / PAC: FUNCn_IN_SEL_CFG pad [4:0], SEL bit 6. */
#define SIG_I2C_SCL 53u
#define SIG_I2C_SDA 54u
#define IN_SEL(pad) ((1u << 6) | (pad))
#endif
#define SIG_FSPICLK 63u
#define SIG_FSPIQ   64u
#define SIG_FSPID   65u
#define SIG_FSPICS0 68u

#define GPIO(o)           REG(GPIO_BASE + (o))
#define GPIO_OUT_W1TS     GPIO(0x08)
#define GPIO_ENABLE_W1TS  GPIO(0x24)
#define GPIO_ENABLE_W1TC  GPIO(0x28)
#define GPIO_PIN(n)       GPIO(0x74 + 4u * (n))
#define GPIO_IN_SEL(sig)  GPIO(0x154 + 4u * (sig))
#define GPIO_OUT_SEL(pad) GPIO(0x554 + 4u * (pad))
#define PAD_DRIVER (1u << 2)

/* IO_MUX: MCU_SEL = 1 (GPIO, i.e. the matrix), FUN_DRV = 2, FUN_IE; pull-up
 * (FUN_WPU, bit 8) only where asked. */
#define IO_MUX_PAD(n) REG(IO_MUX_BASE + 4u + 4u * (n))
#define PAD_MATRIX ((1u << 12) | (2u << 10) | (1u << 9))
#define PAD_WPU (1u << 8)

#define SPI(o)       REG(SPI2_BASE + (o))
#define SPI_CMD      SPI(0x00)
#define SPI_CLOCK    SPI(0x0C)
#define SPI_USER     SPI(0x10)
#define SPI_USER1    SPI(0x14)
#define SPI_MS_DLEN  SPI(0x1C)
#define SPI_MISC     SPI(0x20)
#define SPI_DMA_CONF SPI(0x30)
#define SPI_INT_CLR  SPI(0x38)
#define SPI_INT_RAW  SPI(0x3C)
#define SPI_W(n)     SPI(0x98 + 4u * (n))
#define SPI_SLAVE    SPI(0xE0)
#define SPI_SLAVE1   SPI(0xE4)
#define SPI_CLK_GATE SPI(0xE8)
#define SPI_USR      (1u << 24)
/* SPI_DMA_CONF: DMA_RX_ENA 27, DMA_TX_ENA 28; RX_AFIFO_RST 29,
 * BUF_AFIFO_RST 30, DMA_AFIFO_RST 31. */
#define SPI_DMA_RX_ENA (1u << 27)
#define SPI_DMA_TX_ENA (1u << 28)
#define SPI_AFIFO_RST  (7u << 29)

/* GDMA channel 0, the registers both chips place alike (IN_CONF0 0x70,
 * IN_LINK 0x80, IN_PERI_SEL 0xA0, OUT_LINK 0xE0, OUT_PERI_SEL 0x100);
 * OUT_CONF0 moved on the C6. PERI_SEL 0 = SPI2. LINK: the descriptor
 * address's low 20 bits, START (IN bit 22, OUT bit 21). */
#define GDMA(o)          REG(GDMA_BASE + (o))
#define GDMA_IN_CONF0    GDMA(0x70)
#define GDMA_IN_LINK     GDMA(0x80)
#define GDMA_IN_PERI_SEL GDMA(0xA0)
#define GDMA_OUT_CONF0   GDMA(GDMA_OUT_CONF0_CH0)
#define GDMA_OUT_LINK    GDMA(0xE0)
#define GDMA_OUT_PERI_SEL GDMA(0x100)
#define GDMA_INLINK_START  (1u << 22)
#define GDMA_OUTLINK_START (1u << 21)
/* Linked-list descriptor dw0: owner 31, suc_eof 30, length 23:12, size 11:0. */
#define DESC_OWNER   (1u << 31)
#define DESC_SUC_EOF (1u << 30)
#define DESC_LEN(n)  ((uint32_t)(n) << 12)
/* SRAM the DMA firmware uses by address: the transmit and receive buffers
 * and one descriptor for each direction. */
#define TX_BUF   (SRAM_BASE + 0x200u)
#define RX_BUF   (SRAM_BASE + 0x300u)
#define OUT_DESC (SRAM_BASE + 0x400u)
#define IN_DESC  (SRAM_BASE + 0x410u)

/* One descriptor for each direction over `n` bytes, the SPI's DMA FIFOs
 * reset and DMA enabled for both directions, both links bound to SPI2 and
 * started: esp-idf's spi_hal_prepare_data order. */
static inline void spi_dma_start(uint32_t n) {
    REG(OUT_DESC) = DESC_OWNER | DESC_SUC_EOF | DESC_LEN(n) | n;
    REG(OUT_DESC + 4u) = TX_BUF;
    REG(OUT_DESC + 8u) = 0u;
    REG(IN_DESC) = DESC_OWNER | n;
    REG(IN_DESC + 4u) = RX_BUF;
    REG(IN_DESC + 8u) = 0u;
    GDMA_OUT_PERI_SEL = 0u;
    GDMA_IN_PERI_SEL = 0u;
    GDMA_IN_CONF0 = 1u;                         /* IN_RST */
    GDMA_IN_CONF0 = 0u;
    GDMA_OUT_CONF0 = 1u | (1u << 3);            /* OUT_RST, OUT_EOF_MODE */
    GDMA_OUT_CONF0 = 1u << 3;
    SPI_DMA_CONF = SPI_AFIFO_RST;
    SPI_DMA_CONF = SPI_DMA_RX_ENA | SPI_DMA_TX_ENA;
    GDMA_IN_LINK = GDMA_INLINK_START | (IN_DESC & 0xFFFFFu);
    GDMA_OUT_LINK = GDMA_OUTLINK_START | (OUT_DESC & 0xFFFFFu);
}
#define SPI_UPDATE   (1u << 23)
#define SPI_TRANS_DONE (1u << 12)

#define I2C(o)            REG(I2C0_BASE + (o))
#define I2C_SCL_LOW       I2C(0x00)
#define I2C_CTR           I2C(0x04)
#define I2C_SR            I2C(0x08)
#define I2C_SLAVE_ADDR    I2C(0x10)
#define I2C_FIFO_CONF     I2C(0x18)
#define I2C_DATA          I2C(0x1C)
#define I2C_INT_RAW       I2C(0x20)
#define I2C_INT_CLR       I2C(0x24)
#define I2C_SDA_HOLD      I2C(0x30)
#define I2C_SDA_SAMPLE    I2C(0x34)
#define I2C_SCL_HIGH      I2C(0x38)
#define I2C_START_HOLD    I2C(0x40)
#define I2C_RSTART_SETUP  I2C(0x44)
#define I2C_STOP_HOLD     I2C(0x48)
#define I2C_STOP_SETUP    I2C(0x4C)
#define I2C_CLK_CONF      I2C(0x54)
#define I2C_COMD(n)       I2C(0x58 + 4u * (n))
#define I2C_STRETCH_CONF  I2C(0x84)
#define I2C_CONF_UPGATE   (1u << 11)
#define I2C_TRANS_START   (1u << 5)
#define I2C_MS_MODE       (1u << 4)
#define I2C_FORCE_OUT     3u          /* SDA_FORCE_OUT | SCL_FORCE_OUT */
#define I2C_CLK_EN        (1u << 8)
#define INT_RXFIFO_WM     (1u << 0)
#define INT_END_DETECT    (1u << 3)
#define INT_TRANS_COMPLETE (1u << 7)
#define INT_NACK          (1u << 10)
#define INT_SLAVE_STRETCH (1u << 16)
#define STRETCH_EN        (1u << 10)
#define STRETCH_CLR       (1u << 11)
/* COMD: opcode [13:11] (RSTART 6, WRITE 1, READ 3, STOP 2, END 4),
 * ack_value 10, ack_exp 9, ack_check_en 8, byte_num [7:0]. */
#define CMD(op, n, flags) (((uint32_t)(op) << 11) | (uint32_t)(n) | (flags))
#define OP_RSTART 6u
#define OP_WRITE  1u
#define OP_READ   3u
#define OP_STOP   2u
#define ACK_CHECK (1u << 8)
#define NACK_LAST (1u << 10)

/* Peripheral clocks. C3: SYSTEM_PERIP_CLK_EN0 / PERIP_RST_EN0 bit 6 SPI2,
 * bit 7 I2C_EXT0. C6: PCR_SPI2_CONF / PCR_I2C0_CONF bit 0 CLK_EN, bit 1
 * RST_EN. */
static inline void enable_clocks(void) {
#ifdef CHIP_C6
    REG(0x600960C0u) = 1u;          /* PCR_SPI2_CONF: clock on, out of reset */
    REG(0x60096020u) = 1u;          /* PCR_I2C0_CONF */
#else
    REG(0x600C0010u) |= (1u << 6) | (1u << 7);
    REG(0x600C0018u) &= ~((1u << 6) | (1u << 7));
#endif
}

/* Route `pad` as a matrix output of `sig` (output stage on). */
static inline void matrix_out(unsigned pad, unsigned sig, uint32_t pad_bits) {
    IO_MUX_PAD(pad) = PAD_MATRIX | pad_bits;
    GPIO_OUT_SEL(pad) = sig;
    GPIO_ENABLE_W1TS = 1u << pad;
}

/* Route input `sig` from `pad` (output stage off). */
static inline void matrix_in(unsigned pad, unsigned sig) {
    IO_MUX_PAD(pad) = PAD_MATRIX;
    GPIO_ENABLE_W1TC = 1u << pad;
    GPIO_IN_SEL(sig) = IN_SEL(pad);
}

/* About a microsecond at 160 MHz. */
static inline void delay_us(uint32_t us) {
    for (volatile uint32_t i = 0; i < us * 32u; ++i) {}
}

int main(void);

__attribute__((naked, section(".text.start"))) void _start(void) {
    __asm__ volatile(
#ifdef CHIP_C6
        "li sp, 0x40880000\n"
#else
        "li sp, 0x3FCD0000\n"
#endif
        "call main\n"
        "1: j 1b\n");
}
#endif
