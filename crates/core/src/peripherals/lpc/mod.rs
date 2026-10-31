// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! NXP LPC-style blocks used by the i.MX RT500 (MIMXRT595S): the FLEXCOMM
//! USART, the high-speed GPIO, the RT500 CLKCTL/RSTCTL/IOPCTL banks and the
//! RT500 OS event timer.

pub mod flexcomm_usart;
pub mod gpio;
pub mod ostimer;
pub mod rt5xx_ctl;
