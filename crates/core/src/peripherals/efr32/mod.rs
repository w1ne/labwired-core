// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Silicon Labs EFR32/EFM32 **Series 2** peripheral models (xG21/xG24/xG26 …).
//!
//! Series 2 is not Series 1 with new base addresses. The register blocks were
//! re-laid-out (the USART case is already documented in
//! [`crate::peripherals::uart`]), every peripheral gained the SET/CLR/TGL
//! alias window (see [`labwired_config::AtomicAliasFlavour`]), and the CMU
//! stopped being a divider tree with enable bits scattered through it and
//! became three flat `CLKEN` registers plus per-peripheral clock selectors.
//! So the Series-0/1 models imported from Renode stay where they are; this
//! module is for the parts whose silicon is genuinely different.
//!
//! Register facts here are taken from the vendor CMSIS device headers
//! (`simplicity_sdk` tag `sisdk-2025.6`,
//! `platform/Device/SiliconLabs/EFR32MG26/Include/`) and from the EFR32xG26
//! Reference Manual rev 1.0. Silicon Labs publishes **no SVD** for this
//! family, so those headers are the authoritative machine-readable source and
//! the reason this chip has no `debug_schema` entries.

pub mod cmu;
pub mod gpio_exti;
pub mod iadc;
pub mod timer;
