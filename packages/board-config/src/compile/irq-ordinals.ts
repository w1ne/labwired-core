/**
 * ESP32-S3 ets_isr_source_t ordinals for manifest IRQ validation.
 *
 * Values verified against core Rust source in this worktree:
 * - i2c0 = 42: core/crates/core/src/peripherals/esp32s3/i2c.rs
 *              constant I2C0_INTR_SOURCE_ID (== ETS_I2C_EXT0_INTR_SOURCE)
 * - i2c1 = 43: core/crates/core/src/peripherals/esp32s3/i2c.rs
 *              constant I2C1_INTR_SOURCE_ID (== ETS_I2C_EXT1_INTR_SOURCE)
 *
 * Only entries with a verified source-code citation land here.
 * Unknown peripherals are silently skipped (allowlist-style).
 */
export const ESP32S3_IRQ_SOURCES: Record<string, number> = {
  i2c0: 42, // ETS_I2C_EXT0_INTR_SOURCE — core/crates/core/src/peripherals/esp32s3/i2c.rs:I2C0_INTR_SOURCE_ID
  i2c1: 43, // ETS_I2C_EXT1_INTR_SOURCE — core/crates/core/src/peripherals/esp32s3/i2c.rs:I2C1_INTR_SOURCE_ID
};
