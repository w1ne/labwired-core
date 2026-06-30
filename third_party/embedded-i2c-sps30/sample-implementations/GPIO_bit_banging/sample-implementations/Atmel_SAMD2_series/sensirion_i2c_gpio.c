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

#include "sensirion_i2c_gpio.h"
#include "sensirion_common.h"
#include "sensirion_config.h"

#include <asf.h>

/*
 * We use the following names for the two I2C signal lines:
 * SCL for the clock line
 * SDA for the data line
 *
 * Both lines must be equipped with pull-up resistors appropriate to the bus
 * frequency.
 */

ioport_pin_t scl_port = IOPORT_CREATE_PIN(IOPORT_PORTA, 9);
ioport_pin_t sda_port = IOPORT_CREATE_PIN(IOPORT_PORTA, 8);

/**
 * Initialize all hard- and software components that are needed to set the
 * SDA and SCL pins.
 */
void sensirion_i2c_gpio_init_pins(void) {
    ioport_enable_pin(scl_port);
    ioport_enable_pin(sda_port);
    sensirion_i2c_gpio_SDA_in();
    sensirion_i2c_gpio_SCL_in();
}

/**
 * Release all resources initialized by sensirion_i2c_gpio_init_pins()
 */
void sensirion_i2c_gpio_release_pins(void) {
}

/**
 * Configure the SDA pin as an input. With an external pull-up resistor the line
 * should be left floating, without external pull-up resistor, the input must be
 * configured to use the internal pull-up resistor.
 */
void sensirion_i2c_gpio_SDA_in(void) {
    ioport_set_pin_dir(sda_port, IOPORT_DIR_INPUT);
    ioport_set_pin_mode(sda_port, IOPORT_MODE_OPEN_DRAIN);
}

/**
 * Configure the SDA pin as an output and drive it low or set to logical false.
 */
void sensirion_i2c_gpio_SDA_out(void) {
    ioport_set_pin_dir(sda_port, IOPORT_DIR_OUTPUT);
    ioport_set_pin_level(sda_port, false);
}

/**
 * Read the value of the SDA pin.
 * @returns 0 if the pin is low and 1 otherwise.
 */
uint8_t sensirion_i2c_gpio_SDA_read(void) {
    return ioport_get_pin_level(sda_port);
}

/**
 * Configure the SCL pin as an input. With an external pull-up resistor the line
 * should be left floating, without external pull-up resistor, the input must be
 * configured to use the internal pull-up resistor.
 */
void sensirion_i2c_gpio_SCL_in(void) {
    ioport_set_pin_dir(scl_port, IOPORT_DIR_INPUT);
    ioport_set_pin_mode(scl_port, IOPORT_MODE_OPEN_DRAIN);
}

/**
 * Configure the SCL pin as an output and drive it low or set to logical false.
 */
void sensirion_i2c_gpio_SCL_out(void) {
    ioport_set_pin_dir(scl_port, IOPORT_DIR_OUTPUT);
    ioport_set_pin_level(scl_port, false);
}

/**
 * Read the value of the SCL pin.
 * @returns 0 if the pin is low and 1 otherwise.
 */
uint8_t sensirion_i2c_gpio_SCL_read(void) {
    return ioport_get_pin_level(scl_port);
}

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
void sensirion_i2c_gpio_sleep_usec(uint32_t useconds) {
    /* TODO: IMPLEMENT */
}
