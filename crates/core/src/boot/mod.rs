// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Boot-path implementations for each supported chip.
//!
//! Each submodule provides a `fast_boot` function that takes an ELF byte slice,
//! a `SystemBus` with all peripherals already mapped, and a CPU; loads the ELF
//! segments via the bus; synthesises the post-bootloader CPU state; and
//! returns a `BootSummary` describing what was loaded.

pub mod esp32s3;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BootError {
    #[error("ELF parse error: {0}")]
    ElfParse(String),
    #[error("ELF segment vaddr 0x{addr:08x} (size {size}) is outside any mapped peripheral")]
    SegmentOutsideMap { addr: u32, size: usize },
    #[error("flash-XIP page table overflow: tried to map {requested} pages (max 64)")]
    TooManyXipPages { requested: usize },
    #[error("no stack top: ELF symbol _stack_start_cpu0 not found and no fallback supplied")]
    // Reserved: emitted by future fast_boot variants where stack_top_fallback is optional.
    NoStackTop,
    #[error("simulator error during boot: {0}")]
    Sim(#[from] crate::SimulationError),
}

pub type BootResult<T> = Result<T, BootError>;
