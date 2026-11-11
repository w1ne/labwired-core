#include <stdint.h>

/* WCH CH32V003 register addresses from the CH32V003 reference manual. */
#define RCC_CFGR0       (*(volatile uint32_t *)0x40021004u)
#define RCC_APB1PCENR   (*(volatile uint32_t *)0x4002101Cu)
#define RCC_APB2PCENR   (*(volatile uint32_t *)0x40021018u)
#define GPIOC_CFGLR     (*(volatile uint32_t *)0x40011000u)
#define GPIOD_CFGLR     (*(volatile uint32_t *)0x40011400u)

#define USART1_STATR    (*(volatile uint32_t *)0x40013800u)
#define USART1_DATAR    (*(volatile uint32_t *)0x40013804u)
#define USART1_BRR      (*(volatile uint32_t *)0x40013808u)
#define USART1_CTLR1    (*(volatile uint32_t *)0x4001380Cu)

/* CH32V003 I2C1 has the legacy CR1/CR2/DR/SR1/SR2 register layout. */
#define I2C1_CR1        (*(volatile uint32_t *)0x40005400u)
#define I2C1_CR2        (*(volatile uint32_t *)0x40005404u)
#define I2C1_OAR1       (*(volatile uint32_t *)0x40005408u)
#define I2C1_DR         (*(volatile uint32_t *)0x40005410u)
#define I2C1_SR1        (*(volatile uint32_t *)0x40005414u)
#define I2C1_SR2        (*(volatile uint32_t *)0x40005418u)
#define I2C1_CCR        (*(volatile uint32_t *)0x4000541Cu)
#define I2C1_TRISE      (*(volatile uint32_t *)0x40005420u)

#define RCC_GPIOCEN     (1u << 4)
#define RCC_GPIODEN     (1u << 5)
#define RCC_USART1EN    (1u << 14)
#define RCC_I2C1EN      (1u << 21)

#define USART_STATR_TC  (1u << 6)
#define USART_STATR_TXE (1u << 7)
#define USART_CTLR1_TE  (1u << 3)
#define USART_CTLR1_UE  (1u << 13)

#define I2C_CR1_PE      (1u << 0)
#define I2C_CR1_START   (1u << 8)
#define I2C_CR1_STOP    (1u << 9)
#define I2C_CR1_ACK     (1u << 10)
#define I2C_SR1_SB      (1u << 0)
#define I2C_SR1_ADDR    (1u << 1)
#define I2C_SR1_BTF     (1u << 2)
#define I2C_SR1_RXNE    (1u << 6)
#define I2C_SR1_TXE     (1u << 7)

#define BME280_ADDRESS  0x77u
#define POLL_LIMIT      100000u

static void uart_putc(char c) {
    while ((USART1_STATR & USART_STATR_TXE) == 0u) {
    }
    USART1_DATAR = (uint8_t)c;
}

static void uart_puts(const char *s) {
    while (*s != '\0') {
        uart_putc(*s++);
    }
}

static int wait_i2c_flag(uint32_t flag) {
    for (uint32_t n = 0; n < POLL_LIMIT; ++n) {
        if ((I2C1_SR1 & flag) != 0u) {
            return 1;
        }
    }
    return 0;
}

static int i2c_start(void) {
    I2C1_CR1 |= I2C_CR1_START;
    return wait_i2c_flag(I2C_SR1_SB);
}

static int i2c_send_address(uint8_t address, int read) {
    I2C1_DR = (uint32_t)((address << 1) | (read ? 1u : 0u));
    if (!wait_i2c_flag(I2C_SR1_ADDR)) {
        return 0;
    }

    /* A one-byte receive NACKs before clearing ADDR, then asks for STOP. */
    if (read) {
        I2C1_CR1 &= ~I2C_CR1_ACK;
    }
    (void)I2C1_SR2;
    if (read) {
        I2C1_CR1 |= I2C_CR1_STOP;
    }
    return 1;
}

static int i2c_write_byte(uint8_t value) {
    if (!wait_i2c_flag(I2C_SR1_TXE)) {
        return 0;
    }
    I2C1_DR = value;
    return wait_i2c_flag(I2C_SR1_BTF);
}

static int bme280_read_id(uint8_t *id) {
    if (!i2c_start() || !i2c_send_address(BME280_ADDRESS, 0) ||
        !i2c_write_byte(0xD0u) || !i2c_start() ||
        !i2c_send_address(BME280_ADDRESS, 1) ||
        !wait_i2c_flag(I2C_SR1_RXNE)) {
        I2C1_CR1 |= I2C_CR1_STOP;
        return 0;
    }
    *id = (uint8_t)I2C1_DR;
    return 1;
}

int main(void) {
    /* The CH32V003 reset clock is 24 MHz HSI; keep the smoke on that path. */
    RCC_CFGR0 &= 0xF8FF0000u;
    RCC_APB2PCENR |= RCC_GPIOCEN | RCC_GPIODEN | RCC_USART1EN;
    RCC_APB1PCENR |= RCC_I2C1EN;

    /* Board header defaults: PC1=SDA, PC2=SCL, alternate-function open drain. */
    GPIOC_CFGLR = (GPIOC_CFGLR & ~((0xFu << 4) | (0xFu << 8))) |
                  (0xFu << 4) | (0xFu << 8);
    /* USART1 default TX is PD5; 50 MHz alternate-function push-pull. */
    GPIOD_CFGLR = (GPIOD_CFGLR & ~(0xFu << 20)) | (0xBu << 20);

    USART1_BRR = 0x00D0u; /* 24 MHz / 115200 baud. */
    USART1_CTLR1 = USART_CTLR1_UE | USART_CTLR1_TE;

    /* Standard-mode I2C at 100 kHz from the 24 MHz APB1 reset clock. */
    I2C1_CR2 = 24u;
    I2C1_CCR = 120u;
    I2C1_TRISE = 25u;
    I2C1_OAR1 = 1u << 14;
    I2C1_CR1 = I2C_CR1_PE | I2C_CR1_ACK;

    uint8_t id = 0u;
    if (!bme280_read_id(&id)) {
        uart_puts("CH32V003_BME280_I2C_ERROR\r\n");
    } else if (id != 0x60u) {
        uart_puts("CH32V003_BME280_BAD_ID\r\n");
    } else {
        uart_puts("CH32V003_BME280_OK ID=0x60 I2C1=PC1/PC2\r\n");
    }

    while ((USART1_STATR & USART_STATR_TC) == 0u) {
    }
    for (;;) {
        __asm__ volatile("nop");
    }
}
