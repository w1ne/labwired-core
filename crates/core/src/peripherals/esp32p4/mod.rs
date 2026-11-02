// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-P4 (RISC-V HP core) peripheral wiring.
//!
//! The P4 is a single-architecture RISC-V part for the engine's purposes: the
//! official `esp32p4.svd` declares the CPU as **RV32IMAFC** (a RISC-V HP core),
//! and the LX7-class low-power core is a separate, non-target hart — the same
//! HP-core-only scope `esp32c6.yaml` takes. `Arch::RiscV` and
//! `MachineFamily::RiscV` already cover it, and `RiscVCoreProfile::for_chip`
//! returns the ESP32 vendor CSR profile for any name containing "esp32".
//!
//! The P4 shares a lot of Espressif IP with the C3/C6/S3 — the UART head
//! register map, the SYSTIMER layout and the TIMG head are all offset-identical
//! — so this module is deliberately thin and wires the existing models instead
//! of cloning them:
//!
//! | P4 type             | served by                                       |
//! |---------------------|-------------------------------------------------|
//! | `esp32p4_uart`      | [`crate::peripherals::esp_uart::EspUart`] (head map shared with C3/C6/S3; built with the C6 reg map) |
//! | `esp32p4_systimer`  | [`crate::peripherals::esp_systimer::Systimer`] (layout shared with S3/C3) |
//! | `esp32p4_timg`      | [`crate::peripherals::esp32::timg::Timg`] (head shared with C3/C6; MWDT path NOT armed) |
//! | `declarative` gpio / io_mux / clkrst / interrupt_core0 | SVD-derived descriptors in `configs/peripherals/esp32p4/` |
//!
//! ## What is P4-only and NOT modelled
//!
//! Unlike the C6 there is **no `clock:` gate**: the P4's clock/reset block
//! (`HP_SYS_CLKRST`) has a different register layout from the C6 `PCR`, its
//! tree is not modelled, and no peripheral is gated on `CLK_EN0` at L1.
//!
//! The P4 GPIO is a **two-bank** block (OUT1/ENABLE1/IN1/STATUS1), unlike the
//! C3/C6 single-bank head, so it is a register-backed `declarative` descriptor
//! with no electrical enforcement.
//!
//! Everything else — the LP subsystem, MIPI-DSI/CSI, USB 2.0, GDMA/DMA2D/AXI
//! DMA, H.264/JPEG/PPA/ISP, EMAC, SD/MMC, eFuse and every radio — is not
//! declared at all, so unmapped windows fault loudly. The honest list is in
//! `examples/esp32p4-guition-jc4880p443/KNOWN_LIMITATIONS.md`.

pub mod factory;
pub mod uart;
