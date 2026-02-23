// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use memmap2::MmapMut;
use std::fs::OpenOptions;

/// Shared memory transport for co-simulation.
pub struct ShmTransport {
    pub mmap: MmapMut,
}

impl ShmTransport {
    pub fn new(path: &str, size: usize) -> anyhow::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)?;
        file.set_len(size as u64)?;
        let mmap = unsafe { MmapMut::map_mut(&file)? };
        Ok(Self { mmap })
    }
}
