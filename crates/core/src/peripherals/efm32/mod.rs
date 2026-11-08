// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//! Silicon Labs EFM32 **Series 0** (Gecko G/GG/LG/WG/TG/ZG/HG) peripheral
//! models. The Series-0 register maps are not the Series-2 ones in
//! [`super::efr32`]; the GPIO port lives in [`super::gpio`] as the
//! `Efm32s0` layout.
pub mod timer;
