/*
 * nRF54L15 smart-ring I²C sensor-probe firmware.
 *
 * Extends the nrf54l15-dk smoke firmware from "boot + UART banner" to a real
 * I²C bring-up: after boot it configures TWIM21 (I²C master, EasyDMA) on the
 * ring's sensor bus (SCL=P1.02, SDA=P1.03) and does a genuine register read of
 * each of the four sensors' identity register, printing the address and the
 * byte(s) returned over UARTE20.
 *
 * The bus and addresses match configs/systems/smart-ring.yaml:
 *
 *   id      part       addr   id register            expected
 *   imu     BMI270     0x68   0x00 CHIP_ID           0x24
 *   ppg     MAX30102   0x57   0xFF PART_ID           0x15
 *   temp    TMP117     0x48   0x0F DEVICE_ID (16b)   0x0117  (BE: 0x01 0x17)
 *   haptic  DRV2605    0x5A   0x00 STATUS            devid=7 in bits[7:5]
 *
 * Every read is a real TWIM transaction: a one-byte TX of the register pointer,
 * a repeated START (LASTTX_DMA_RX_START short), a RX of the ID byte(s), then
 * STOP (LASTRX_STOP short). The sensor MODELS answer these transactions; a byte
 * that comes back matching the datasheet ID is proof the transaction reached
 * the modelled slave, not a stub. The firmware also reports the ACK: an
 * unpopulated address NACKs (ERRORSRC.ANACK / EVENTS_ERROR), so `ack=Y` means
 * the model acknowledged its address on the bus.
 *
 * Everything is polled; no interrupts. The part boots on the internal HFOSC.
 */
#include <stdint.h>

#include "nrf54l15.h"

/*
 * EasyDMA reads/writes these buffers over the bus, so they MUST live in RAM,
 * not RRAM — the same rule the smoke firmware's tx_buf follows. They are
 * written at runtime, which forces them into .data/.bss (RAM).
 */
static char    tx_buf[96];   /* UARTE TX source         */
static uint8_t i2c_reg;      /* TWIM TX: register ptr   */
static uint8_t i2c_rx[4];    /* TWIM RX: returned bytes */

/* ── UARTE20 console ─────────────────────────────────────────────────────── */

static uint32_t str_len(const char *s)
{
    uint32_t n = 0;
    while (s[n] != '\0') {
        n++;
    }
    return n;
}

static void uarte_init(void)
{
    UARTE_PSEL_TXD(UARTE20_BASE) = UARTE20_PIN_TXD;
    UARTE_PSEL_RXD(UARTE20_BASE) = UARTE20_PIN_RXD;
    UARTE_BAUDRATE(UARTE20_BASE) = UARTE_BAUD_115200;
    UARTE_ENABLE(UARTE20_BASE)   = UARTE_ENABLE_UARTE;
}

/* DMA `len` bytes out of tx_buf (which the caller has already filled). */
static void uarte_flush(uint32_t len)
{
    if (len == 0) {
        return;
    }
    UARTE_EVENTS_DMA_TX_END(UARTE20_BASE)  = 0;
    UARTE_DMA_TX_PTR(UARTE20_BASE)         = (uint32_t)(uintptr_t)tx_buf;
    UARTE_DMA_TX_MAXCNT(UARTE20_BASE)      = len;
    UARTE_TASKS_DMA_TX_START(UARTE20_BASE) = 1;
    while (UARTE_EVENTS_DMA_TX_END(UARTE20_BASE) == 0) {
    }
    UARTE_EVENTS_DMA_TX_END(UARTE20_BASE)  = 0;
    UARTE_TASKS_DMA_TX_STOP(UARTE20_BASE)  = 1;
}

static void uart_puts(const char *s)
{
    uint32_t len = str_len(s);
    uint32_t n   = (len < sizeof(tx_buf)) ? len : (uint32_t)sizeof(tx_buf);
    for (uint32_t i = 0; i < n; i++) {
        tx_buf[i] = s[i];
    }
    uarte_flush(n);
}

/* Line builder: append into tx_buf at *pos, flush once per line. */
static void app_str(uint32_t *pos, const char *s)
{
    uint32_t i = 0;
    while (s[i] != '\0' && *pos < sizeof(tx_buf)) {
        tx_buf[(*pos)++] = s[i++];
    }
}

static char hex_digit(uint8_t nib)
{
    return (nib < 10) ? (char)('0' + nib) : (char)('a' + (nib - 10));
}

static void app_hex8(uint32_t *pos, uint8_t v)
{
    app_str(pos, "0x");
    if (*pos < sizeof(tx_buf)) tx_buf[(*pos)++] = hex_digit((v >> 4) & 0xF);
    if (*pos < sizeof(tx_buf)) tx_buf[(*pos)++] = hex_digit(v & 0xF);
}

static void app_hex16(uint32_t *pos, uint16_t v)
{
    app_str(pos, "0x");
    for (int shift = 12; shift >= 0; shift -= 4) {
        if (*pos < sizeof(tx_buf)) {
            tx_buf[(*pos)++] = hex_digit((uint8_t)((v >> shift) & 0xF));
        }
    }
}

/* ── TWIM21 I²C ───────────────────────────────────────────────────────────── */

static void twim_init(void)
{
    TWIM_PSEL_SCL(TWIM21_BASE)  = TWIM21_PIN_SCL;
    TWIM_PSEL_SDA(TWIM21_BASE)  = TWIM21_PIN_SDA;
    TWIM_FREQUENCY(TWIM21_BASE) = TWIM_FREQUENCY_K400;
    TWIM_ENABLE(TWIM21_BASE)    = TWIM_ENABLE_ENABLED;
}

/*
 * Read `n` bytes (n <= sizeof(i2c_rx)) from register `reg` of the I²C slave at
 * 7-bit `addr`, using the canonical write-pointer / repeated-START / read /
 * STOP sequence driven by shorts. Returns 1 if the slave ACKed (no bus error),
 * 0 on NACK (no device at that address).
 */
static int twim_read_reg(uint8_t addr, uint8_t reg, uint8_t n)
{
    /* Sentinel: if the slave never responds, the RX buffer keeps 0xEE so a
     * non-response is visibly distinct from a real 0x00 ID. */
    for (uint8_t i = 0; i < n && i < sizeof(i2c_rx); i++) {
        i2c_rx[i] = 0xEE;
    }
    i2c_reg = reg;

    TWIM_ADDRESS(TWIM21_BASE) = addr;
    TWIM_SHORTS(TWIM21_BASE)  = TWIM_SHORT_LASTTX_DMA_RX_START | TWIM_SHORT_LASTRX_STOP;

    TWIM_DMA_TX_PTR(TWIM21_BASE)    = (uint32_t)(uintptr_t)&i2c_reg;
    TWIM_DMA_TX_MAXCNT(TWIM21_BASE) = 1;
    TWIM_DMA_RX_PTR(TWIM21_BASE)    = (uint32_t)(uintptr_t)i2c_rx;
    TWIM_DMA_RX_MAXCNT(TWIM21_BASE) = n;

    /* Clear stale events / errors before arming the transfer. */
    TWIM_EVENTS_STOPPED(TWIM21_BASE)    = 0;
    TWIM_EVENTS_ERROR(TWIM21_BASE)      = 0;
    TWIM_EVENTS_DMA_RX_END(TWIM21_BASE) = 0;
    TWIM_ERRORSRC(TWIM21_BASE)          = TWIM_ERRORSRC_ANACK; /* write-1-clear */

    /* Kick TX; the LASTTX short chains into the RX leg, LASTRX short STOPs. */
    TWIM_TASKS_DMA_TX_START(TWIM21_BASE) = 1;

    /* Both success and NACK raise EVENTS_STOPPED, so this terminates either
     * way. Bounded to avoid a hang if the model ever failed to complete. */
    for (uint32_t spin = 0; spin < 1000000u; spin++) {
        if (TWIM_EVENTS_STOPPED(TWIM21_BASE) != 0) {
            break;
        }
    }

    int acked = (TWIM_EVENTS_ERROR(TWIM21_BASE) == 0) &&
                ((TWIM_ERRORSRC(TWIM21_BASE) & TWIM_ERRORSRC_ANACK) == 0);

    TWIM_TASKS_STOP(TWIM21_BASE) = 1;
    return acked;
}

/* Probe one 8-bit-ID sensor and print a report line. */
static void probe8(const char *label, uint8_t addr, uint8_t reg, uint8_t expect)
{
    int acked = twim_read_reg(addr, reg, 1);

    uint32_t p = 0;
    app_str(&p, label);
    app_str(&p, " addr=");
    app_hex8(&p, addr);
    app_str(&p, " reg=");
    app_hex8(&p, reg);
    app_str(&p, " -> id=");
    app_hex8(&p, i2c_rx[0]);
    app_str(&p, " ack=");
    app_str(&p, acked ? "Y" : "N");
    app_str(&p, (i2c_rx[0] == expect) ? " [OK]" : " [MISMATCH]");
    app_str(&p, "\r\n");
    uarte_flush(p);
}

/* Probe the TMP117: a 16-bit big-endian DEVICE_ID (MSB then LSB). */
static void probe_tmp117(void)
{
    int acked = twim_read_reg(0x48, 0x0F, 2);
    uint16_t id = ((uint16_t)i2c_rx[0] << 8) | i2c_rx[1];

    uint32_t p = 0;
    app_str(&p, "temp   TMP117   addr=");
    app_hex8(&p, 0x48);
    app_str(&p, " reg=");
    app_hex8(&p, 0x0F);
    app_str(&p, " -> id=");
    app_hex16(&p, id);
    app_str(&p, " ack=");
    app_str(&p, acked ? "Y" : "N");
    app_str(&p, (id == 0x0117) ? " [OK]" : " [MISMATCH]");
    app_str(&p, "\r\n");
    uarte_flush(p);
}

int main(void)
{
    uarte_init();
    twim_init();

    uart_puts("smart-ring nRF54L15 I2C sensor probe\r\n");
    uart_puts("TWIM21@0x500C7000 SCL=P1.02 SDA=P1.03\r\n");

    /* imu / ppg / haptic carry an 8-bit ID; temp is 16-bit (handled apart). */
    probe8("imu    BMI270   ", 0x68, 0x00, 0x24);
    probe8("ppg    MAX30102 ", 0x57, 0xFF, 0x15);
    probe_tmp117();
    /* DRV2605 STATUS bits[7:5] are the DEVICE_ID (7 = DRV2605L); the reset
     * STATUS byte is therefore 0xE0. */
    probe8("haptic DRV2605  ", 0x5A, 0x00, 0xE0);

    uart_puts("probe done\r\n");

    for (;;) {
    }
}
