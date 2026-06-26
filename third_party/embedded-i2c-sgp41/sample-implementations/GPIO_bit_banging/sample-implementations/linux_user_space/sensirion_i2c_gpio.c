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

/* Enable usleep function */
#define _DEFAULT_SOURCE

#include <fcntl.h>     /* close, open */
#include <stdio.h>     /* fprintf, perror, stderr */
#include <stdlib.h>    /* exit */
#include <string.h>    /* strlen */
#include <sys/types.h> /* mode_t */
#include <unistd.h>    /* access, lseek, read, usleep */

#include "sensirion_common.h"
#include "sensirion_config.h"
#include "sensirion_i2c_gpio.h"

/*
 * We use the following names for the two I2C signal lines:
 * SCL for the clock line
 * SDA for the data line
 *
 * Both lines must be equipped with pull-up resistors appropriate to the bus
 * frequency.
 */
#define GPIO_PIN_SCL 12
#define GPIO_PIN_SDA 13
#define GPIO_DIR "/sys/class/gpio/"

#define __str(x) #x
#define GPIO_PIN_STR(p) __str(p)
#define GPIO_PIN_SCL_STR GPIO_PIN_STR(GPIO_PIN_SCL)
#define GPIO_PIN_SDA_STR GPIO_PIN_STR(GPIO_PIN_SDA)
#define GPIO_ID "gpio"
#define GPIO(p) GPIO_ID __str(p)
#define GPIO_PATH(p) GPIO_DIR p
#define GPIO_EXPORT_PATH GPIO_PATH("export")
#define GPIO_SCL_DIR GPIO_PATH(GPIO(GPIO_PIN_SCL))
#define GPIO_SDA_DIR GPIO_PATH(GPIO(GPIO_PIN_SDA))
#define GPIO_SCL_PATH(d) GPIO_SCL_DIR d
#define GPIO_SDA_PATH(d) GPIO_SDA_DIR d
#define GPIO_SCL_DIRECTION GPIO_SCL_PATH("/direction")
#define GPIO_SDA_DIRECTION GPIO_SDA_PATH("/direction")
#define GPIO_SCL_VALUE GPIO_SCL_PATH("/value")
#define GPIO_SDA_VALUE GPIO_SDA_PATH("/value")

#define GPIO_DIRECTION_IN "in"
#define GPIO_DIRECTION_OUT "out"
#define GPIO_LOW 0

static int scl_dir_fd;
static int scl_val_fd;
static int sda_dir_fd;
static int sda_val_fd;

static int open_or_exit(const char* path, int flags) {
    int fd = open(path, flags);
    if (fd < 0) {
        perror(NULL);
        fprintf(stderr, "Error opening %s (mode %d)\n", path, flags);
        exit(-1);
    }
    return fd;
}

static void rev_or_exit(int fd) {
    if (lseek(fd, 0, SEEK_SET) < 0) {
        perror("Error seeking gpio");
        exit(-1);
    }
}

static void write_or_exit(int fd, const char* buf) {
    size_t len = strlen(buf);

    ssize_t w = write(fd, buf, len);

    /* Adapted from stackoverflow answer by Stephen Canon
       See: https://www.stackoverflow.com/a/16086724 */
    if (w < 0 || (size_t)w != len) {
        perror("Error writing");
        exit(-1);
    }
}

static void gpio_export(const char* path, const char* export_pin) {
    int fd;

    if (access(path, F_OK) == -1) {
        fd = open_or_exit(GPIO_EXPORT_PATH, O_WRONLY);
        write_or_exit(fd, export_pin);
        close(fd);
    }
}

static void gpio_set_value(int fd, int value) {
    char buf[] = {'0', '\0'};

    buf[0] += value;
    rev_or_exit(fd);
    write_or_exit(fd, buf);
}

static void gpio_set_direction(int fd, const char* dir) {
    rev_or_exit(fd);
    write_or_exit(fd, dir);
}

static uint8_t gpio_get_value(int fd) {
    char c;

    rev_or_exit(fd);
    if (read(fd, &c, 1) != 1) {
        perror("Error reading GPIO value");
        exit(-1);
    }
    return c == '1';
}

/**
 * Initialize all hard- and software components that are needed to set the
 * SDA and SCL pins.
 */
void sensirion_i2c_gpio_init_pins(void) {
    gpio_export(GPIO_SCL_DIR, GPIO_PIN_SCL_STR);
    gpio_export(GPIO_SDA_DIR, GPIO_PIN_SDA_STR);

    scl_dir_fd = open_or_exit(GPIO_SCL_DIRECTION, O_WRONLY);
    scl_val_fd = open_or_exit(GPIO_SCL_VALUE, O_RDWR);
    sda_dir_fd = open_or_exit(GPIO_SDA_DIRECTION, O_WRONLY);
    sda_val_fd = open_or_exit(GPIO_SDA_VALUE, O_RDWR);
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
    gpio_set_direction(sda_dir_fd, GPIO_DIRECTION_IN);
}

/**
 * Configure the SDA pin as an output and drive it low or set to logical false.
 */
void sensirion_i2c_gpio_SDA_out(void) {
    gpio_set_direction(sda_dir_fd, GPIO_DIRECTION_OUT);
    gpio_set_value(sda_val_fd, GPIO_LOW);
}

/**
 * Read the value of the SDA pin.
 * @returns 0 if the pin is low and 1 otherwise.
 */
uint8_t sensirion_i2c_gpio_SDA_read(void) {
    return gpio_get_value(sda_val_fd);
}

/**
 * Configure the SCL pin as an input. With an external pull-up resistor the line
 * should be left floating, without external pull-up resistor, the input must be
 * configured to use the internal pull-up resistor.
 */
void sensirion_i2c_gpio_SCL_in(void) {
    gpio_set_direction(scl_dir_fd, GPIO_DIRECTION_IN);
}

/**
 * Configure the SCL pin as an output and drive it low or set to logical false.
 */
void sensirion_i2c_gpio_SCL_out(void) {
    gpio_set_direction(scl_dir_fd, GPIO_DIRECTION_OUT);
    gpio_set_value(scl_val_fd, GPIO_LOW);
}

/**
 * Read the value of the SCL pin.
 * @returns 0 if the pin is low and 1 otherwise.
 */
uint8_t sensirion_i2c_gpio_SCL_read(void) {
    return gpio_get_value(scl_val_fd);
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
    usleep(useconds);
}
