/*
 * Copyright (c) 2018, Sensirion AG
 * All rights reserved.
 *
 * Redistribution and use in source and binary forms, with or without
 * modification, are permitted provided that the following conditions are met:
 *
 * * Redistributions of source code must retain the above copyright notice, this
 *   list of conditions and the following disclaimer.
 *
 * * Redistributions in binary form must reproduce the above copyright notice,
 *   this list of conditions and the following disclaimer in the documentation
 *   and/or other materials provided with the distribution.
 *
 * * Neither the name of Sensirion AG nor the names of its
 *   contributors may be used to endorse or promote products derived from
 *   this software without specific prior written permission.
 *
 * THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
 * AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
 * IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
 * ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE
 * LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
 * CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
 * SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
 * INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
 * CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
 * ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
 * POSSIBILITY OF SUCH DAMAGE.
 */

#include "sensirion_i2c_hal.h"
#include "sensirion_common.h"
#include "sensirion_config.h"
#include "sensirion_i2c.h"
#include "sensirion_i2c_gpio.h"

#define DELAY_USEC (SENSIRION_I2C_CLOCK_PERIOD_USEC / 2)

/**
 * Declaration of static helpers.
 */
static int8_t sensirion_i2c_gpio_write_byte(uint8_t data);
static uint8_t sensirion_i2c_gpio_read_byte(uint8_t ack);
static int8_t sensirion_i2c_gpio_start(void);
static void sensirion_i2c_gpio_stop(void);

/**
 * Select the current i2c bus by index.
 * All following i2c operations will be directed at that bus.
 *
 * THE IMPLEMENTATION IS OPTIONAL ON SINGLE-BUS SETUPS (all sensors on the same
 * bus)
 *
 * @param bus_idx   Bus index to select
 * @returns         0 on success, an error code otherwise
 */
int16_t sensirion_i2c_hal_select_bus(uint8_t bus_idx) {
    return NO_ERROR; /* not relevant for software I2C */
}

/**
 * Initialize all hard- and software components that are needed for the I2C
 * communication.
 */
void sensirion_i2c_hal_init(void) {
    sensirion_i2c_gpio_init_pins();
    sensirion_i2c_gpio_SCL_in();
    sensirion_i2c_gpio_SDA_in();
}

/**
 * Release all resources initialized by sensirion_i2c_hal_init().
 */
void sensirion_i2c_hal_free(void) {
    sensirion_i2c_gpio_SCL_in();
    sensirion_i2c_gpio_SDA_in();
    sensirion_i2c_gpio_release_pins();
}

/**
 * Execute one read transaction on the I2C bus, reading a given number of bytes.
 * If the device does not acknowledge the read command, an error shall be
 * returned.
 *
 * @param address 7-bit I2C address to read from
 * @param data    pointer to the buffer where the data is to be stored
 * @param count   number of bytes to read from I2C and store in the buffer
 * @returns 0 on success, error code otherwise
 */
int8_t sensirion_i2c_hal_read(uint8_t address, uint8_t* data, uint8_t count) {
    int8_t ret;
    uint8_t send_ack;
    uint16_t i;

    ret = sensirion_i2c_gpio_start();
    if (ret != NO_ERROR)
        return ret;

    ret = sensirion_i2c_gpio_write_byte((address << 1) | 1);
    if (ret != NO_ERROR) {
        sensirion_i2c_gpio_stop();
        return ret;
    }
    for (i = 0; i < count; i++) {
        send_ack = i < (count - 1); /* last byte must be NACK'ed */
        data[i] = sensirion_i2c_gpio_read_byte(send_ack);
    }

    sensirion_i2c_gpio_stop();
    return NO_ERROR;
}

/**
 * Execute one write transaction on the I2C bus, sending a given number of
 * bytes. The bytes in the supplied buffer must be sent to the given address. If
 * the slave device does not acknowledge any of the bytes, an error shall be
 * returned.
 *
 * @param address 7-bit I2C address to write to
 * @param data    pointer to the buffer containing the data to write
 * @param count   number of bytes to read from the buffer and send over I2C
 * @returns 0 on success, error code otherwise
 */
int8_t sensirion_i2c_hal_write(uint8_t address, const uint8_t* data,
                               uint8_t count) {
    int8_t ret;
    uint16_t i;

    ret = sensirion_i2c_gpio_start();
    if (ret != NO_ERROR)
        return ret;

    ret = sensirion_i2c_gpio_write_byte(address << 1);
    if (ret != NO_ERROR) {
        sensirion_i2c_gpio_stop();
        return ret;
    }
    for (i = 0; i < count; i++) {
        ret = sensirion_i2c_gpio_write_byte(data[i]);
        if (ret != NO_ERROR) {
            sensirion_i2c_gpio_stop();
            break;
        }
    }
    sensirion_i2c_gpio_stop();
    return ret;
}

/**
 * Sleep for a given number of microseconds. The function should delay the
 * execution for at least the given time, but may also sleep longer.
 *
 * Despite the unit, a <10 millisecond precision is sufficient.
 *
 * @param useconds the sleep time in microseconds
 */
void sensirion_i2c_hal_sleep_usec(uint32_t useconds) {
    sensirion_i2c_gpio_sleep_usec(useconds);
}

/**
 * The following functions are static helpers.
 */

static int8_t sensirion_wait_while_clock_stretching(void) {
    /* Maximal timeout of 150ms (SCD30) in sleep polling cycles */
    uint32_t timeout_cycles = 150000 / SENSIRION_I2C_CLOCK_PERIOD_USEC;

    while (--timeout_cycles) {
        if (sensirion_i2c_gpio_SCL_read())
            return NO_ERROR;
        sensirion_i2c_gpio_sleep_usec(SENSIRION_I2C_CLOCK_PERIOD_USEC);
    }

    return I2C_BUS_ERROR;
}

static int8_t sensirion_i2c_gpio_write_byte(uint8_t data) {
    int8_t nack, i;
    for (i = 7; i >= 0; i--) {
        sensirion_i2c_gpio_SCL_out();
        if ((data >> i) & 0x01)
            sensirion_i2c_gpio_SDA_in();
        else
            sensirion_i2c_gpio_SDA_out();
        sensirion_i2c_gpio_sleep_usec(DELAY_USEC);
        sensirion_i2c_gpio_SCL_in();
        sensirion_i2c_gpio_sleep_usec(DELAY_USEC);
        if (sensirion_wait_while_clock_stretching())
            return I2C_BUS_ERROR;
    }
    sensirion_i2c_gpio_SCL_out();
    sensirion_i2c_gpio_SDA_in();
    sensirion_i2c_gpio_sleep_usec(DELAY_USEC);
    sensirion_i2c_gpio_SCL_in();
    if (sensirion_wait_while_clock_stretching())
        return I2C_BUS_ERROR;
    nack = (sensirion_i2c_gpio_SDA_read() != 0);
    sensirion_i2c_gpio_SCL_out();

    return nack;
}

static uint8_t sensirion_i2c_gpio_read_byte(uint8_t ack) {
    int8_t i;
    uint8_t data = 0;
    sensirion_i2c_gpio_SDA_in();
    for (i = 7; i >= 0; i--) {
        sensirion_i2c_gpio_sleep_usec(DELAY_USEC);
        sensirion_i2c_gpio_SCL_in();
        if (sensirion_wait_while_clock_stretching())
            return 0xFF; /* return 0xFF on error */
        data |= (sensirion_i2c_gpio_SDA_read() != 0) << i;
        sensirion_i2c_gpio_SCL_out();
    }
    if (ack)
        sensirion_i2c_gpio_SDA_out();
    else
        sensirion_i2c_gpio_SDA_in();
    sensirion_i2c_gpio_sleep_usec(DELAY_USEC);
    sensirion_i2c_gpio_SCL_in();
    sensirion_i2c_gpio_sleep_usec(DELAY_USEC);
    if (sensirion_wait_while_clock_stretching())
        return 0xFF; /* return 0xFF on error */
    sensirion_i2c_gpio_SCL_out();
    sensirion_i2c_gpio_SDA_in();

    return data;
}

static int8_t sensirion_i2c_gpio_start(void) {
    sensirion_i2c_gpio_SCL_in();
    if (sensirion_wait_while_clock_stretching())
        return I2C_BUS_ERROR;

    sensirion_i2c_gpio_SDA_out();
    sensirion_i2c_gpio_sleep_usec(DELAY_USEC);
    sensirion_i2c_gpio_SCL_out();
    sensirion_i2c_gpio_sleep_usec(DELAY_USEC);
    return NO_ERROR;
}

static void sensirion_i2c_gpio_stop(void) {
    sensirion_i2c_gpio_SDA_out();
    sensirion_i2c_gpio_sleep_usec(DELAY_USEC);
    sensirion_i2c_gpio_SCL_in();
    sensirion_i2c_gpio_sleep_usec(DELAY_USEC);
    sensirion_i2c_gpio_SDA_in();
    sensirion_i2c_gpio_sleep_usec(DELAY_USEC);
}
