// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::DmaRequest;

/// Subset of `PeripheralTickResult` returned by `Peripheral::on_event`.
/// The bus reuses the same fan-out machinery (IRQ pend, mmio writes, PPI
/// fired_events, DMA) so no new side-channels are introduced.
#[derive(Debug, Clone, Default)]
pub struct EventResult {
    pub raise_irq: Option<u32>,
    pub explicit_irqs: Vec<u32>,
    pub system_exception: Option<u32>,
    pub mmio_writes: Vec<(u32, u32)>,
    pub fired_events: Vec<u32>,
    pub dma_requests: Vec<DmaRequest>,
}
