// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::signals::InterruptLine;
use std::fmt::Debug;

/// Trait representing a generic interrupt controller.
///
/// This trait allows different architectures (ARM NVIC, RISC-V CLIC, Xtensa L1/L2)
/// to be plugged into the same modular system.
pub trait InterruptController: Debug + Send + Sync {
    /// Signal the controller that an interrupt line has changed.
    fn set_interrupt_pending(&self, irq: u32, pending: bool);

    /// Check if a specific interrupt is enabled and pending.
    fn is_interrupt_active(&self, irq: u32) -> bool;

    /// Acknowledge an interrupt, usually called by the CPU at the start of an ISR.
    fn acknowledge_interrupt(&self) -> Option<u32>;

    /// Complete an interrupt, usually called by the CPU after an ISR finishes.
    fn complete_interrupt(&self, irq: u32);
}

/// A bridge that connects `InterruptLine` signals to an `InterruptController`.
pub struct InterruptBridge<'a> {
    controller: &'a dyn InterruptController,
}

impl<'a> InterruptBridge<'a> {
    pub fn new(controller: &'a dyn InterruptController) -> Self {
        Self { controller }
    }

    pub fn update(&self, irq: u32, line: &InterruptLine) {
        self.controller
            .set_interrupt_pending(irq, line.is_pending());
    }
}
