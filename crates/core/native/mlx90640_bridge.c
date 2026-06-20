// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// See mlx90640_bridge.h. This file implements the Melexis driver's I2C
// platform shim on top of Rust callbacks, and a thin decode entry point the
// LabWired integration test invokes.

#include "mlx90640_bridge.h"

#include <MLX90640_API.h>
#include <MLX90640_I2C_Driver.h>
#include <string.h>

// ── MLX90640 I2C platform shim (the part normally board-specific) ───────────
// On real hardware these poke the MCU's I2C controller. Here they forward to
// the Rust device model via the callbacks declared in the header — so the
// vendor driver runs byte-identically against the simulated bus.

void MLX90640_I2CInit(void) {}

int MLX90640_I2CGeneralReset(void) { return 0; }

void MLX90640_I2CFreqSet(int freq) { (void)freq; }

int MLX90640_I2CRead(uint8_t slaveAddr, uint16_t startAddress,
                     uint16_t nMemAddressRead, uint16_t* data) {
    return lw_mlx_rust_read_words(slaveAddr, startAddress, nMemAddressRead, data);
}

int MLX90640_I2CWrite(uint8_t slaveAddr, uint16_t writeAddress, uint16_t data) {
    return lw_mlx_rust_write_word(slaveAddr, writeAddress, data);
}

// ── Decode entry points ─────────────────────────────────────────────────────

// Persisted params across calls would require state; keep it simple and
// re-extract each decode (the driver does this once per session on target, but
// re-extracting is harmless and keeps the bridge stateless).
static int extract(uint8_t slave, paramsMLX90640* params) {
    static uint16_t eeData[832];
    int err = MLX90640_DumpEE(slave, eeData);
    if (err != 0) {
        return err;
    }
    return MLX90640_ExtractParameters(eeData, params);
}

int lw_mlx_decode_scene(uint8_t slave, float emissivity, float tr,
                        float* result_out) {
    static paramsMLX90640 params;
    int perr = extract(slave, &params);

    // Chess mode splits the image across two subpages; decode both so all 768
    // pixels are populated.
    static uint16_t frameData[834];
    for (int sub = 0; sub < 2; sub++) {
        int got = MLX90640_GetFrameData(slave, frameData);
        if (got < 0) {
            return got; // frame fetch error
        }
        // GetFrameData returns the subpage it fetched; CalculateTo writes only
        // the pixels belonging to frameData[833]. Both subpages carry the same
        // scene in this model, so two fetches reconstruct the full field.
        MLX90640_CalculateTo(frameData, &params, emissivity, tr, result_out);
    }
    return perr;
}

int lw_mlx_decode_ta_vdd(uint8_t slave, float* ta_out, float* vdd_out) {
    static paramsMLX90640 params;
    int perr = extract(slave, &params);
    static uint16_t frameData[834];
    int got = MLX90640_GetFrameData(slave, frameData);
    if (got < 0) {
        return got;
    }
    *vdd_out = MLX90640_GetVdd(frameData, &params);
    *ta_out = MLX90640_GetTa(frameData, &params);
    return perr;
}
