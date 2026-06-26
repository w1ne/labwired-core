/*
 * I2C-Generator: 0.3.0
 * Yaml Version: 0.1.0
 * Template Version: 0.7.0-62-g3d691f9
 */
/*
 * Copyright (c) 2021, Sensirion AG
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

#include <stdio.h>  // printf

#include "sensirion_common.h"
#include "sensirion_i2c_hal.h"
#include "sgp41_i2c.h"
// #include <inttypes.h>

/*
 * TO USE CONSOLE OUTPUT (PRINTF) YOU MAY NEED TO ADAPT THE INCLUDE ABOVE OR
 * DEFINE IT ACCORDING TO YOUR PLATFORM:
 * #define printf(...)
 */

int main(void) {
    int16_t error = 0;

    sensirion_i2c_hal_init();

    uint16_t serial_number[3];

    error = sgp41_get_serial_number(serial_number);
    if (error) {
        printf("Error executing sgp41_get_serial_number(): %i\n", error);
    } else {
        // printf("Serial number: %" PRIu64 "\n",
        //         (((uint64_t)serial_number[0]) << 32) |
        //         (((uint64_t)serial_number[1]) << 16) |
        //         ((uint64_t)serial_number[2]));
        printf("serial: 0x%04x%04x%04x\n", serial_number[0], serial_number[1],
               serial_number[2]);
    }

    uint16_t test_result;

    error = sgp41_execute_self_test(&test_result);
    if (error) {
        printf("Error executing sgp41_execute_self_test(): %i\n", error);
    } else {
        printf("Test result: %u\n", test_result);
    }

    // Start Measurement

    // Parameters for deactivated humidity compensation:
    uint16_t default_rh = 0x8000;
    uint16_t default_t = 0x6666;

    // sgp41 conditioning during 10 seconds before measuring
    // WARNING: To avoid damage to the sensing material the conditioning must
    // not exceed 10s!
    for (int i = 0; i < 10; i++) {
        uint16_t sraw_voc;

        sensirion_i2c_hal_sleep_usec(1000000);

        error = sgp41_execute_conditioning(default_rh, default_t, &sraw_voc);
        if (error) {
            printf("Error executing sgp41_execute_conditioning(): "
                   "%i\n",
                   error);
        } else {
            printf("SRAW VOC: %u\n", sraw_voc);
            printf("SRAW NOx: conditioning\n");
        }
    }

    for (int i = 0; i < 60; i++) {
        uint16_t sraw_voc;
        uint16_t sraw_nox;

        sensirion_i2c_hal_sleep_usec(1000000);

        error = sgp41_measure_raw_signals(default_rh, default_t, &sraw_voc,
                                          &sraw_nox);
        if (error) {
            printf("Error executing sgp41_measure_raw_signals(): "
                   "%i\n",
                   error);
        } else {
            printf("SRAW VOC: %u\n", sraw_voc);
            printf("SRAW NOx: %u\n", sraw_nox);
        }
    }

    return 0;
}
