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
    /// Phase 2B.3b (issue #192): pend the peripheral's *own* configured NVIC
    /// line (`PeripheralEntry::irq`). Mirrors the legacy `tick()` path's
    /// `irq: bool`, which the bus maps to `p.irq` — a peripheral that doesn't
    /// know its own IRQ number (e.g. the shared `Uart`) sets this instead of
    /// `raise_irq`.
    pub raise_own_irq: bool,
    /// Phase 2B.3b: DMA TX/RX signal IDs to route exactly as the legacy
    /// `PeripheralTickResult::dma_signals` path does (by source-peripheral
    /// name → target DMA channel). Empty for peripherals that don't drive DMA.
    pub dma_signals: Vec<u32>,
    /// Phase 2B.3b: re-arm this same event after `delay_ticks` (using the
    /// just-fired `event_token`). Lets a level-triggered peripheral perpetuate
    /// itself while it has active work, then stop by returning `None`. The
    /// `Machine` drain loop owns the idx needed to reschedule.
    pub reschedule_delay: Option<u64>,
}
