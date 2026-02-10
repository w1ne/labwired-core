// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

/// Represents a digital signal level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DigitalLevel {
    #[default]
    Low,
    High,
}

impl From<bool> for DigitalLevel {
    fn from(b: bool) -> Self {
        if b {
            DigitalLevel::High
        } else {
            DigitalLevel::Low
        }
    }
}

impl From<DigitalLevel> for bool {
    fn from(level: DigitalLevel) -> Self {
        match level {
            DigitalLevel::High => true,
            DigitalLevel::Low => false,
        }
    }
}

/// A simple digital signal that can be read or written.
#[derive(Debug, Clone, Default)]
pub struct DigitalSignal {
    level: DigitalLevel,
}

impl DigitalSignal {
    pub fn new(level: DigitalLevel) -> Self {
        Self { level }
    }

    pub fn set(&mut self, level: DigitalLevel) {
        self.level = level;
    }

    pub fn get(&self) -> DigitalLevel {
        self.level
    }
}

/// Represents a specialized signal line for interrupts.
#[derive(Debug, Clone, Default)]
pub struct InterruptLine {
    pending: bool,
}

impl InterruptLine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_pending(&mut self) {
        self.pending = true;
    }

    pub fn clear(&mut self) {
        self.pending = false;
    }

    pub fn is_pending(&self) -> bool {
        self.pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_digital_signal() {
        let mut sig = DigitalSignal::default();
        assert_eq!(sig.get(), DigitalLevel::Low);
        sig.set(DigitalLevel::High);
        assert_eq!(sig.get(), DigitalLevel::High);

        let b: bool = sig.get().into();
        assert!(b);
    }

    #[test]
    fn test_interrupt_line() {
        let mut irq = InterruptLine::new();
        assert!(!irq.is_pending());
        irq.set_pending();
        assert!(irq.is_pending());
        irq.clear();
        assert!(!irq.is_pending());
    }
}
