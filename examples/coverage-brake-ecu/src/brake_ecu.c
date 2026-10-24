// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

// Brake-light ECU, the kind of small safety function coverage is for.
//
// The pedal sensor sends a 5-byte frame over the bus (here: UART):
//
//   [0xA5 sync] [alive counter] [pedal %] [flags] [CRC-8]
//
// Each frame is protected end-to-end like AUTOSAR E2E Profile 1: a CRC-8
// (SAE J1850) over the first four bytes, and a 4-bit alive counter that must
// advance by 1..MAX_DELTA from frame to frame. Three bad frames in a row put
// the ECU in a latched safe state: brake light on until reset.

#include <stdint.h>

#define UART_SR (*(volatile uint8_t *)0x4000C000)
#define UART_DR (*(volatile uint8_t *)0x4000C004)
#define SR_RXNE (1u << 5)

#define SYNC 0xA5
#define MAX_DELTA 2
#define FLAG_EMERGENCY 0x01
#define MAX_ERRORS 3

enum e2e_status { E2E_OK, E2E_CRC_ERROR, E2E_REPEATED, E2E_WRONG_SEQUENCE };

static uint8_t last_counter;
static uint8_t have_counter;
static uint8_t error_count;
static uint8_t safe_state;

static void put(const char *s) {
    while (*s) {
        UART_DR = (uint8_t)*s++;
    }
}

static uint8_t get(void) {
    while ((UART_SR & SR_RXNE) == 0) {
    }
    return UART_DR;
}

// CRC-8 SAE J1850: poly 0x1D, init 0xFF, final XOR 0xFF.
static uint8_t crc8_j1850(const uint8_t *data, int len) {
    uint8_t crc = 0xFF;
    for (int i = 0; i < len; i++) {
        crc ^= data[i];
        for (int bit = 0; bit < 8; bit++) {
            if (crc & 0x80) {
                crc = (uint8_t)((crc << 1) ^ 0x1D);
            } else {
                crc = (uint8_t)(crc << 1);
            }
        }
    }
    return crc ^ 0xFF;
}

static enum e2e_status e2e_check(const uint8_t *frame) {
    if (crc8_j1850(frame, 4) != frame[4]) {
        return E2E_CRC_ERROR;
    }
    uint8_t counter = frame[1] & 0x0F;
    if (!have_counter) {
        have_counter = 1;
        last_counter = counter;
        return E2E_OK;
    }
    uint8_t delta = (uint8_t)((counter - last_counter) & 0x0F);
    if (delta == 0) {
        return E2E_REPEATED;
    }
    last_counter = counter;
    if (delta > MAX_DELTA) {
        return E2E_WRONG_SEQUENCE;
    }
    return E2E_OK;
}

static void handle_frame(const uint8_t *frame) {
    if (safe_state) {
        put("SAFE STATE\n");
        return;
    }

    enum e2e_status status = e2e_check(frame);
    if (status != E2E_OK) {
        switch (status) {
        case E2E_CRC_ERROR:
            put("E2E CRC ERROR\n");
            break;
        case E2E_REPEATED:
            put("E2E REPEATED\n");
            break;
        default:
            put("E2E WRONG SEQUENCE\n");
            break;
        }
        error_count++;
        if (error_count >= MAX_ERRORS) {
            safe_state = 1;
            put("SAFE STATE\n");
        }
        return;
    }
    error_count = 0;

    uint8_t pedal = frame[2];
    if (pedal > 100) {
        put("PEDAL IMPLAUSIBLE\n");
        return;
    }
    if (pedal >= 5 || (frame[3] & FLAG_EMERGENCY)) {
        put("BRAKE ON\n");
    } else {
        put("BRAKE OFF\n");
    }
}

int main(void) {
    last_counter = 0;
    have_counter = 0;
    error_count = 0;
    safe_state = 0;
    put("BRAKE ECU READY\n");

    uint8_t frame[5];
    for (;;) {
        // Resynchronise on the sync byte.
        do {
            frame[0] = get();
        } while (frame[0] != SYNC);
        for (int i = 1; i < 5; i++) {
            frame[i] = get();
        }
        handle_frame(frame);
    }
}
