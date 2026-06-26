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

#ifndef SENSIRION_SW_I2C_H
#define SENSIRION_SW_I2C_H

#include "sensirion_common.h"
#include "sensirion_config.h"

/**
 * The clock period of the i2c bus in microseconds. Increase this, if your GPIO
 * ports cannot support a 200 kHz output rate. (2 * 1 / 10usec == 200Khz)
 *
 * This is only relevant for the sw-i2c HAL (bit-banging on GPIO pins). The
 * pulse length is half the clock period, the number should thus be even.
 */
#define SENSIRION_I2C_CLOCK_PERIOD_USEC 10

#ifdef __cplusplus
extern "C" {
#endif
/**
 * Initialize all hard- and software components that are needed to set the
 * SDA and SCL pins.
 */
void sensirion_i2c_gpio_init_pins(void);

/**
 * Release all resources initialized by sensirion_i2c_gpio_init_pins()
 */
void sensirion_i2c_gpio_release_pins(void);

/**
 * Configure the SDA pin as an input. With an external pull-up resistor the line
 * should be left floating, without external pull-up resistor, the input must be
 * configured to use the internal pull-up resistor.
 */
void sensirion_i2c_gpio_SDA_in(void);

/**
 * Configure the SDA pin as an output and drive it low or set to logical false.
 */
void sensirion_i2c_gpio_SDA_out(void);

/**
 * Read the value of the SDA pin.
 * @returns 0 if the pin is low and 1 otherwise.
 */
uint8_t sensirion_i2c_gpio_SDA_read(void);

/**
 * Configure the SCL pin as an input. With an external pull-up resistor the line
 * should be left floating, without external pull-up resistor, the input must be
 * configured to use the internal pull-up resistor.
 */
void sensirion_i2c_gpio_SCL_in(void);

/**
 * Configure the SCL pin as an output and drive it low or set to logical false.
 */
void sensirion_i2c_gpio_SCL_out(void);

/**
 * Read the value of the SCL pin.
 * @returns 0 if the pin is low and 1 otherwise.
 */
uint8_t sensirion_i2c_gpio_SCL_read(void);

/**
 * Sleep for a given number of microseconds. The function should delay the
 * execution approximately, but no less than, the given time.
 *
 * The precision needed depends on the desired i2c frequency, i.e. should be
 * exact to about half a clock cycle (defined in
 * `SENSIRION_I2C_CLOCK_PERIOD_USEC` in `sensirion_i2c_gpio.h`).
 *
 * Example with 400kHz requires a precision of 1 / (2 * 400kHz) == 1.25usec.
 *
 * @param useconds the sleep time in microseconds
 */
void sensirion_i2c_gpio_sleep_usec(uint32_t useconds);

#ifdef __cplusplus
}
#endif

#endif /* SENSIRION_SW_I2C_H */
