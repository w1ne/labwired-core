// LabWired - ESP32-S2-DevKitC-1-N8R2 + BME280 smoke
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

// Bare-metal ESP32-S2 BME280 smoke. The local twin fast-starts this ELF into
// the S2 IROM/DROM windows; it does not run the mask ROM or flash bootloader.
#include <stdint.h>

#define REG32(address) (*(volatile uint32_t *)(address))

#define UART0_FIFO REG32(0x60000000u)
#define UART0_CLKDIV REG32(0x60000014u)

#define GPIO_BASE 0x60004000u
#define GPIO_ENABLE_W1TS REG32(GPIO_BASE + 0x24u)
#define GPIO_FUNC0_IN_SEL_CFG REG32(GPIO_BASE + 0x130u)
#define GPIO_FUNC0_OUT_SEL_CFG REG32(GPIO_BASE + 0x530u)
#define GPIO_PIN8 REG32(GPIO_BASE + 0x88u + 8u * 4u)
#define GPIO_PIN9 REG32(GPIO_BASE + 0x88u + 9u * 4u)

#define I2C0_BASE 0x60013000u
#define I2C_SCL_LOW_PERIOD REG32(I2C0_BASE + 0x00u)
#define I2C_CTR REG32(I2C0_BASE + 0x04u)
#define I2C_INT_RAW REG32(I2C0_BASE + 0x20u)
#define I2C_INT_CLR REG32(I2C0_BASE + 0x24u)
#define I2C_SCL_HIGH_PERIOD REG32(I2C0_BASE + 0x38u)
#define I2C_SDA_HOLD REG32(I2C0_BASE + 0x30u)
#define I2C_SDA_SAMPLE REG32(I2C0_BASE + 0x34u)
#define I2C_SCL_START_HOLD REG32(I2C0_BASE + 0x40u)
#define I2C_SCL_RSTART_SETUP REG32(I2C0_BASE + 0x44u)
#define I2C_SCL_STOP_HOLD REG32(I2C0_BASE + 0x48u)
#define I2C_SCL_STOP_SETUP REG32(I2C0_BASE + 0x4Cu)
#define I2C_CLK_CONF REG32(I2C0_BASE + 0x54u)
#define I2C_CMD0 REG32(I2C0_BASE + 0x58u)
#define I2C_DATA REG32(I2C0_BASE + 0x1Cu)
#define I2C_FIFO_CONF REG32(I2C0_BASE + 0x18u)

#define I2C_ADDRESS 0x77u
#define CMD_RSTART 0u
#define CMD_WRITE 1u
#define CMD_READ 2u
#define CMD_STOP 3u
#define CMD_ACK_VALUE (1u << 10)
#define FIFO_RX_RST (1u << 12)
#define FIFO_TX_RST (1u << 13)
#define INT_NACK (1u << 10)
#define INT_TRANS_COMPLETE (1u << 7)
#define CTR_MASTER_CLK_EN ((1u << 8) | (1u << 4) | 0x3u)
#define CTR_TRANS_START (1u << 5)
#define SPIN_LIMIT 120000u

static void uart_string(const char *text) {
    while (*text != '\0') {
        UART0_FIFO = (uint32_t)(uint8_t)*text++;
    }
}

__attribute__((noreturn)) static void wait_forever(void) {
    for (;;) {
        __asm__ volatile("nop");
    }
}

static int read_bme280_registers(uint8_t reg, uint8_t *destination, uint32_t count) {
    if (count == 0 || count > 3) {
        return 0;
    }

    I2C_FIFO_CONF = FIFO_RX_RST | FIFO_TX_RST;
    I2C_INT_CLR = 0xFFFFFFFFu;

    // ESP32-S2 TRM §25.2.5 opcodes: RSTART=0, WRITE=1, READ=2, STOP=3.
    I2C_CMD0 = (CMD_RSTART << 11);
    REG32(I2C0_BASE + 0x5Cu) = (CMD_WRITE << 11) | 2u;
    REG32(I2C0_BASE + 0x60u) = (CMD_RSTART << 11);
    REG32(I2C0_BASE + 0x64u) = (CMD_WRITE << 11) | 1u;
    REG32(I2C0_BASE + 0x68u) = (CMD_READ << 11) | count | CMD_ACK_VALUE;
    REG32(I2C0_BASE + 0x6Cu) = (CMD_STOP << 11);

    I2C_DATA = (I2C_ADDRESS << 1);
    I2C_DATA = reg;
    I2C_DATA = (I2C_ADDRESS << 1) | 1u;
    I2C_CTR = CTR_MASTER_CLK_EN | CTR_TRANS_START;

    uint32_t complete = 0;
    for (uint32_t i = 0; i < SPIN_LIMIT; ++i) {
        if ((I2C_INT_RAW & INT_TRANS_COMPLETE) != 0) {
            complete = 1;
            break;
        }
    }
    if (!complete) {
        return 0;
    }

    uint32_t status = I2C_INT_RAW;
    I2C_INT_CLR = status;
    if ((status & INT_NACK) != 0) {
        return 0;
    }
    for (uint32_t i = 0; i < count; ++i) {
        destination[i] = (uint8_t)I2C_DATA;
    }
    return 1;
}

__attribute__((noreturn)) void _start(void) {
    UART0_CLKDIV = 694u; // 80 MHz source / 694 ≈ 115200 baud.

    // S2 GPIO-matrix I2CEXT0 signals are SCL=29 and SDA=30. GPIO8/9 are
    // ordinary exposed header pins on DevKitC-1; output-enable remains owned
    // by GPIO_ENABLE and PAD_DRIVER selects the open-drain pad mode.
    GPIO_FUNC0_OUT_SEL_CFG = 30u;
    REG32(GPIO_BASE + 0x530u + 9u * 4u) = 29u;
    GPIO_FUNC0_IN_SEL_CFG = (1u << 7) | 9u;
    REG32(GPIO_BASE + 0x130u + 30u * 4u) = (1u << 7) | 8u;
    GPIO_PIN8 = 1u << 2;
    GPIO_PIN9 = 1u << 2;
    GPIO_ENABLE_W1TS = (1u << 8) | (1u << 9);

    // Check the matrix setup before issuing a sensor transaction.
    if (GPIO_FUNC0_OUT_SEL_CFG != 30u
        || REG32(GPIO_BASE + 0x530u + 9u * 4u) != 29u
        || GPIO_FUNC0_IN_SEL_CFG != ((1u << 7) | 9u)
        || REG32(GPIO_BASE + 0x130u + 30u * 4u) != ((1u << 7) | 8u)) {
        uart_string("ESP32S2_I2C_MATRIX_FAIL\n");
        wait_forever();
    }

    // 100 kHz timing using the S2 40 MHz XTAL source.
    I2C_CLK_CONF = 0;
    I2C_SCL_LOW_PERIOD = 199u;
    I2C_SCL_HIGH_PERIOD = 199u;
    I2C_SDA_HOLD = 2u;
    I2C_SDA_SAMPLE = 2u;
    I2C_SCL_START_HOLD = 10u;
    I2C_SCL_RSTART_SETUP = 10u;
    I2C_SCL_STOP_HOLD = 10u;
    I2C_SCL_STOP_SETUP = 10u;
    I2C_CTR = CTR_MASTER_CLK_EN;

    uint8_t chip_id = 0;
    if (!read_bme280_registers(0xD0u, &chip_id, 1u) || chip_id != 0x60u) {
        uart_string("ESP32S2_BME280_ID_FAIL\n");
        wait_forever();
    }

    uint8_t temperature[3] = {0, 0, 0};
    if (!read_bme280_registers(0xFAu, temperature, 3u)) {
        uart_string("ESP32S2_BME280_TEMP_FAIL\n");
        wait_forever();
    }
    uint32_t raw_temperature = ((uint32_t)temperature[0] << 12)
        | ((uint32_t)temperature[1] << 4)
        | ((uint32_t)temperature[2] >> 4);
    if (raw_temperature != 0x082DC0u) {
        uart_string("ESP32S2_BME280_VALUE_FAIL\n");
        wait_forever();
    }

    uart_string("ESP32S2_DEVKITC1_N8R2_BME280_OK ID=0x60 TEMP_RAW=0x082DC0 I2C0=GPIO8/9\n");
    wait_forever();
}
