// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C6 (RISC-V HP core) peripheral wiring.
//!
//! The C6 is the C3's successor with a different memory map and a different
//! clock/reset block, but a lot of shared IP: the UART head register map and
//! the GPIO matrix are offset-identical to the C3's. So this module is
//! deliberately thin — it owns the C6-specific *values* (interrupt-matrix
//! source ids, core clock, which model serves each window) and wires the
//! existing models instead of cloning them:
//!
//! | C6 type             | served by                                       |
//! |---------------------|-------------------------------------------------|
//! | `esp32c6_uart`      | [`crate::peripherals::esp_uart::EspUart`] (head map shared with C3/S3) |
//! | `esp32c6_gpio`      | [`crate::peripherals::esp32c3::gpio::Esp32c3Gpio`] (register head shared) |
//! | `esp32c6_pcr`       | [`pcr::Esp32c6Pcr`] — full SVD register map + the `clock:` gate controller |
//! | `esp32c6_gdma`      | [`gdma::Esp32c6Gdma`] — 3-channel C6 GDMA with M2M descriptor walks |
//! | `esp32c6_mwdt`      | [`crate::peripherals::esp32::timg::Timg`] with `with_mwdt` — the shared TIMG head (one GP timer per group) plus the C3/C6 MWDT path; the name carries the `wdt` marker the tier-1 heuristic reads |
//! | `esp32c6_lp_rtc`    | [`lp_timer::Esp32c6LpTimer`] — C6 LP_TIMER main counter + snapshot buffers |
//! | `declarative` io_mux / interrupt_core0 / intpri / hp_sys | SVD-derived descriptors in `configs/peripherals/esp32c6/` |
//!
//! The `interrupt_core0` + `intpri` pair is more than a stub pair: the bus's
//! C3 interrupt fabric (`crates/core/src/bus/routing.rs`) selects its C6
//! register layout from the presence of `intpri` and routes matrix sources
//! through the real enable/priority/threshold gates into the RISC-V core.
//!
//! What is C6-only and NOT modelled (see
//! `examples/esp32c6-devkitc/KNOWN_LIMITATIONS.md`): the LP core, the C6 UART
//! register tail, IO_MUX electrical enforcement, PCR `RST_EN` enforcement and
//! the clock tree behind the dividers, peripheral-coupled DMA, peripheral-
//! sourced matrix interrupts, and every radio.

pub mod extmem;
pub mod factory;
pub mod gdma;
pub mod lp_timer;
pub mod modem_lpcon;
pub mod i2c_ana_mst;
pub mod pcr;
pub mod uart;
