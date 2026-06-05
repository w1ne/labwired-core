// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SVD-driven register-coverage probing for chip models.
//!
//! The probe engine measures, by OBSERVABLE BEHAVIOR alone, whether a model
//! actually implements each register a chip's SVD declares — so the coverage
//! number cannot be inflated by an author declaring a stub "modelled".

pub mod probe;

pub use probe::{probe_peripheral, Access, ProbeReg, ProbeTarget, RegResult, RegStatus};
