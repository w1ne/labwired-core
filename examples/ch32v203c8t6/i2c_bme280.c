/* I2C1 polled and I2C2 interrupt-driven master reads for the CH32V203C8T6.
 *
 * ch32fun supplies the QingKe V4 startup and interrupt vector table.
 * The system file puts one BME280 on I2C1 (PB6 SCL, PB7 SDA, address 0x76)
 * and one on I2C2 (PB10 SCL, PB11 SDA, address 0x77).
 *
 * I2C1, polled: a one-byte read of the chip-id register 0xD0 (the N=1
 * receive sequence: ACK cleared before ADDR), then a three-byte read of the
 * calibration words at 0x88 (the N>2 sequence that ends on BTF).
 * I2C2, interrupt-driven: the same chip-id read, stepped by the I2C2_EV_IRQn
 * 49 handler through SB, ADDR, TXE, BTF and RXNE.
 *
 * Expected: id 0x60 on both buses; dig_T1 = 28528 (0x6F70) and the dig_T2
 * low byte from the BME280 model's calibration set.
 */
#include "ch32fun.h"
#include <stdint.h>

#define BME280_I2C1_ADDR 0x76u
#define BME280_I2C2_ADDR 0x77u
#define BME280_REG_ID    0xD0u
#define BME280_REG_CALIB 0x88u

#define POLL_LIMIT 200000u

enum { EV_IDLE, EV_ADDR_W, EV_REG, EV_RESTART, EV_ADDR_R, EV_DATA, EV_DONE };

static volatile uint32_t ev_state = EV_IDLE;
static volatile uint32_t ev_irqs;
static volatile uint8_t ev_byte;

__attribute__((interrupt)) void I2C2_EV_IRQHandler(void) {
    uint16_t star1 = I2C2->STAR1;
    ev_irqs++;
    if ((star1 & I2C_STAR1_SB) != 0) {
        if (ev_state == EV_ADDR_W) {
            I2C2->DATAR = (uint16_t)(BME280_I2C2_ADDR << 1);
        } else {
            ev_state = EV_ADDR_R;
            I2C2->DATAR = (uint16_t)((BME280_I2C2_ADDR << 1) | 1u);
        }
    } else if ((star1 & I2C_STAR1_ADDR) != 0) {
        if (ev_state == EV_ADDR_W) {
            (void)I2C2->STAR2;
            ev_state = EV_REG;
            I2C2->DATAR = BME280_REG_ID;
        } else {
            /* One byte to receive: NACK it, release ADDR, then STOP. RXNE
             * is a buffer event, so unmask it again. */
            I2C2->CTLR1 &= (uint16_t)~I2C_CTLR1_ACK;
            I2C2->CTLR2 |= I2C_CTLR2_ITBUFEN;
            (void)I2C2->STAR2;
            I2C2->CTLR1 |= I2C_CTLR1_STOP;
            ev_state = EV_DATA;
        }
    } else if (ev_state == EV_REG && (star1 & I2C_STAR1_BTF) != 0) {
        ev_state = EV_RESTART;
        I2C2->CTLR1 |= I2C_CTLR1_START;
    } else if (ev_state == EV_DATA && (star1 & I2C_STAR1_RXNE) != 0) {
        ev_byte = (uint8_t)I2C2->DATAR;
        I2C2->CTLR2 &= (uint16_t)~(I2C_CTLR2_ITEVTEN | I2C_CTLR2_ITBUFEN);
        ev_state = EV_DONE;
    } else if (ev_state == EV_REG || ev_state == EV_RESTART) {
        /* TXE while the register byte shifts out, or BTF until the repeated
         * START lands: mask the buffer interrupt and wait for the event. */
        I2C2->CTLR2 &= (uint16_t)~I2C_CTLR2_ITBUFEN;
    }
}

static void uart_putc(char byte) {
    while ((USART1->STATR & USART_FLAG_TXE) == 0) {
    }
    USART1->DATAR = (uint8_t)byte;
}

static void uart_puts(const char *text) {
    while (*text != '\0') {
        uart_putc(*text++);
    }
}

static void uart_puthex8(uint8_t value) {
    static const char digits[] = "0123456789abcdef";
    uart_putc(digits[value >> 4]);
    uart_putc(digits[value & 0xFu]);
}

static void uart_putu(uint32_t value) {
    char digits[10];
    uint32_t n = 0;
    do {
        digits[n++] = (char)('0' + value % 10u);
        value /= 10u;
    } while (value != 0 && n < sizeof(digits));
    while (n != 0) {
        uart_putc(digits[--n]);
    }
}

static void i2c_init(I2C_TypeDef *i2c) {
    /* 100 kHz standard mode from the 72 MHz PCLK1 (HCLK 144 MHz / 2). */
    i2c->CTLR2 = 36u;
    i2c->CKCFGR = 360u;
    i2c->CTLR1 = I2C_CTLR1_PE;
}

static int wait_star1(I2C_TypeDef *i2c, uint16_t flag) {
    for (uint32_t i = 0; i < POLL_LIMIT; ++i) {
        if ((i2c->STAR1 & flag) != 0) {
            return 1;
        }
    }
    return 0;
}

/* Write the register pointer, then a repeated START in receive mode.
 * Returns with ADDR set and not yet cleared. */
static int i2c1_address_for_read(uint8_t reg) {
    I2C1->CTLR1 |= I2C_CTLR1_START;
    if (!wait_star1(I2C1, I2C_STAR1_SB)) {
        return 0;
    }
    I2C1->DATAR = (uint16_t)(BME280_I2C1_ADDR << 1);
    if (!wait_star1(I2C1, I2C_STAR1_ADDR)) {
        return 0;
    }
    (void)I2C1->STAR2;
    I2C1->DATAR = reg;
    if (!wait_star1(I2C1, I2C_STAR1_BTF)) {
        return 0;
    }
    I2C1->CTLR1 |= I2C_CTLR1_START;
    if (!wait_star1(I2C1, I2C_STAR1_SB)) {
        return 0;
    }
    I2C1->DATAR = (uint16_t)((BME280_I2C1_ADDR << 1) | 1u);
    return wait_star1(I2C1, I2C_STAR1_ADDR);
}

static int i2c1_read1(uint8_t reg, uint8_t *out) {
    if (!i2c1_address_for_read(reg)) {
        return 0;
    }
    I2C1->CTLR1 &= (uint16_t)~I2C_CTLR1_ACK;
    (void)I2C1->STAR2;
    I2C1->CTLR1 |= I2C_CTLR1_STOP;
    if (!wait_star1(I2C1, I2C_STAR1_RXNE)) {
        return 0;
    }
    *out = (uint8_t)I2C1->DATAR;
    return 1;
}

static int i2c1_read3(uint8_t reg, uint8_t *out) {
    I2C1->CTLR1 |= I2C_CTLR1_ACK;
    if (!i2c1_address_for_read(reg)) {
        return 0;
    }
    (void)I2C1->STAR2;
    /* Byte 0 in DATAR, byte 1 in the shift register: NACK byte 2. */
    if (!wait_star1(I2C1, I2C_STAR1_BTF)) {
        return 0;
    }
    I2C1->CTLR1 &= (uint16_t)~I2C_CTLR1_ACK;
    out[0] = (uint8_t)I2C1->DATAR;
    I2C1->CTLR1 |= I2C_CTLR1_STOP;
    out[1] = (uint8_t)I2C1->DATAR;
    if (!wait_star1(I2C1, I2C_STAR1_RXNE)) {
        return 0;
    }
    out[2] = (uint8_t)I2C1->DATAR;
    return 1;
}

int main(void) {
    uint8_t id1 = 0;
    uint8_t calib[3] = {0, 0, 0};

    SystemInit();

    RCC->APB2PCENR |= RCC_APB2Periph_GPIOA | RCC_APB2Periph_GPIOB | RCC_APB2Periph_USART1;
    RCC->APB1PCENR |= RCC_APB1Periph_I2C1 | RCC_APB1Periph_I2C2;

    /* PA9 = USART1 TX, 115200 baud from the 144 MHz PCLK2. */
    GPIOA->CFGHR = (GPIOA->CFGHR & ~(0xFu << 4)) | (0xBu << 4);
    USART1->BRR = 0x04E2u;
    USART1->CTLR1 = USART_CTLR1_UE | USART_CTLR1_TE;

    /* PB6/PB7 (I2C1 SCL/SDA) and PB10/PB11 (I2C2 SCL/SDA): 10 MHz
     * alternate-function open-drain. */
    GPIOB->CFGLR = (GPIOB->CFGLR & ~(0xFFu << 24)) | (0xDDu << 24);
    GPIOB->CFGHR = (GPIOB->CFGHR & ~(0xFFu << 8)) | (0xDDu << 8);

    i2c_init(I2C1);
    i2c_init(I2C2);

    int ok1 = i2c1_read1(BME280_REG_ID, &id1) && i2c1_read3(BME280_REG_CALIB, calib);

    NVIC_EnableIRQ(I2C2_EV_IRQn);
    ev_state = EV_ADDR_W;
    I2C2->CTLR2 |= I2C_CTLR2_ITEVTEN | I2C_CTLR2_ITBUFEN;
    I2C2->CTLR1 |= I2C_CTLR1_START;
    for (uint32_t i = 0; i < POLL_LIMIT && ev_state != EV_DONE; ++i) {
    }

    uart_puts(ok1 ? "I2C1 id=" : "I2C1_TIMEOUT id=");
    uart_puthex8(id1);
    uart_puts(" dig_T1=");
    uart_putu((uint32_t)calib[0] | ((uint32_t)calib[1] << 8));
    uart_puts(" T2l=");
    uart_puthex8(calib[2]);
    uart_puts(ev_state == EV_DONE ? " I2C2 id=" : " I2C2_TIMEOUT id=");
    uart_puthex8(ev_byte);
    uart_puts(" ev_irqs=");
    uart_putu(ev_irqs);
    uart_puts("\r\n");

    for (;;) {
    }
}
