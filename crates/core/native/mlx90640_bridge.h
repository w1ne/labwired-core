// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// Bridge between the REAL Melexis MLX90640 driver and the LabWired MLX90640
// device model. The driver's I2C platform shim (MLX90640_I2CRead/Write) is
// implemented here and routes to Rust callbacks that poke the model exactly as
// on-target firmware would poke the ESP32-C3 I2C controller.

#ifndef LW_MLX90640_BRIDGE_H
#define LW_MLX90640_BRIDGE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

// ── Rust-provided callbacks (implemented in the integration test) ───────────
// Read `n` 16-bit words starting at `start_addr` from the model's register
// space, writing them into `out` (already byte-swapped to host order).
// Returns 0 on success.
int lw_mlx_rust_read_words(uint8_t slave, uint16_t start_addr, uint16_t n, uint16_t* out);
// Write a single 16-bit word to `addr`. Returns 0 on success.
int lw_mlx_rust_write_word(uint8_t slave, uint16_t addr, uint16_t value);

// ── Bridge-provided API the Rust test calls ─────────────────────────────────
// Run the full official decode path against the model:
//   DumpEE -> ExtractParameters -> GetFrameData (both subpages) -> CalculateTo
// Writes 768 per-pixel °C into `result_out`. `emissivity` and `tr` are the
// usual MLX90640_CalculateTo arguments. Returns the ExtractParameters error
// code (0 == OK); decode proceeds regardless so callers can inspect partials.
int lw_mlx_decode_scene(uint8_t slave, float emissivity, float tr, float* result_out);

// Decode just Ta and VDD from a freshly-fetched frame (diagnostics).
int lw_mlx_decode_ta_vdd(uint8_t slave, float* ta_out, float* vdd_out);

#ifdef __cplusplus
}
#endif

#endif
