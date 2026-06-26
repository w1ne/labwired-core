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

#include <delay.h>
#include <i2c_master.h>

#include "sensirion_common.h"
#include "sensirion_config.h"
#include "sensirion_i2c_hal.h"

struct i2c_master_module i2c_master_instance;

void sensirion_i2c_hal_init(void) {
    /* initialize config structure and software module */
    struct i2c_master_config config_i2c_master;
    i2c_master_get_config_defaults(&config_i2c_master);

    /* change buffer timeout to something longer */
    config_i2c_master.buffer_timeout = 10000;

    /* initialize and enable device with config. */
    i2c_master_init(&i2c_master_instance, SERCOM2, &config_i2c_master);
    i2c_master_enable(&i2c_master_instance);
}

/**
 * Release all resources initialized by sensirion_i2c_hal_init().
 */
void sensirion_i2c_hal_free(void) {
}

int8_t sensirion_i2c_hal_read(uint8_t address, uint8_t* data, uint8_t count) {
    struct i2c_master_packet packet = {
        .address = address,
        .data_length = count,
        .data = (uint8_t*)data,
        .ten_bit_address = false,
        .high_speed = false,
    };
    return i2c_master_read_packet_wait(&i2c_master_instance, &packet);
}

int8_t sensirion_i2c_hal_write(uint8_t address, const uint8_t* data,
                               uint8_t count) {
    struct i2c_master_packet packet = {
        .address = address,
        .data_length = count,
        .data = (uint8_t*)data,
        .ten_bit_address = false,
        .high_speed = false,
    };
    return i2c_master_write_packet_wait_no_stop(&i2c_master_instance, &packet);
}

void sensirion_i2c_hal_sleep_usec(uint32_t useconds) {
    delay_us(useconds);
}
