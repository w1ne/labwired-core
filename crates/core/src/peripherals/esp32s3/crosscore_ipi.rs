// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 cross-core software interrupt registers
//! (`SYSTEM_CPU_INTR_FROM_CPU_0..3`, `0x600C_0030..0x600C_0040`).
//!
//! These four registers are the SMP cross-core doorbell. ESP-IDF's
//! `crosscore_int_ll_trigger_interrupt(core)` writes a non-zero value to
//! `SYSTEM_CPU_INTR_FROM_CPU_{core}_REG`, which asserts the *level*
//! interrupt source `FROM_CPU_INTR{core}` on the **other** core. That
//! core's `esp_crosscore_isr` runs, clears the register (writes 0), and
//! services the reason mask (yield / IPC). Without this doorbell the
//! FreeRTOS SMP scheduler can never yield a remote core or run a pinned
//! task on it — the deadlock that strands `loopTask`.
//!
//! ## Faithful modeling
//!
//! Each register is a plain R/W latch. Because the sources are *level*
//! (held high while the register is non-zero, per `soc/interrupts.h`:
//! "interrupt N generated from a CPU, level"), we re-assert them every
//! tick: `tick()` emits source ID `79 + n` for every `FROM_CPU_n`
//! register that is currently non-zero. Routing to the correct core is
//! handled downstream by the interrupt matrix — core 0 binds source 79
//! on its CORE0 map half, core 1 binds source 80 on its CORE1 half (see
//! [`crate::peripherals::esp32s3::intmatrix`]). When the receiving core's
//! ISR writes 0, the next tick stops asserting the source.
//!
//! Source numbering (esp-idf `soc/esp32s3/interrupts.h`):
//!   FROM_CPU_INTR0 = 79, FROM_CPU_INTR1 = 80, INTR2 = 81, INTR3 = 82.

use crate::{Peripheral, PeripheralTickResult, SimResult};

/// MMIO base: `SYSTEM_CPU_INTR_FROM_CPU_0_REG` (`DR_REG_SYSTEM_BASE + 0x30`).
pub const BASE: u64 = 0x600C_0030;
/// Covers FROM_CPU_0..3 (4 words).
pub const SIZE: u64 = 0x10;

/// `FROM_CPU_INTR0` interrupt source ID; `FROM_CPU_INTR{n}` = `BASE + n`.
const FROM_CPU_SOURCE_BASE: u32 = 79;

#[derive(Debug, Default)]
pub struct Esp32s3CrossCoreIpi {
    /// Latched value of FROM_CPU_0..3. Non-zero = the source is asserting.
    from_cpu: [u32; 4],
}

impl Esp32s3CrossCoreIpi {
    pub fn new() -> Self {
        Self::default()
    }

    fn word(&self, offset: u64) -> u32 {
        let idx = ((offset & !3) / 4) as usize;
        self.from_cpu.get(idx).copied().unwrap_or(0)
    }
}

impl Peripheral for Esp32s3CrossCoreIpi {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.word(offset);
        Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let idx = ((offset & !3) / 4) as usize;
        if let Some(w) = self.from_cpu.get_mut(idx) {
            let byte_off = (offset & 3) * 8;
            *w &= !(0xFFu32 << byte_off);
            *w |= (value as u32) << byte_off;
        }
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.word(offset))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let idx = ((offset & !3) / 4) as usize;
        if let Some(w) = self.from_cpu.get_mut(idx) {
            let was = *w;
            *w = value;
            if std::env::var("LABWIRED_IPI_DEBUG").is_ok() && was == 0 && value != 0 {
                eprintln!(
                    "[ipi] FROM_CPU_{idx} doorbell rung (→ assert source {})",
                    FROM_CPU_SOURCE_BASE + idx as u32
                );
            }
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // Level sources: re-assert FROM_CPU_n's source every tick while its
        // doorbell register is non-zero. The intmatrix routes each to the
        // core that bound it; the receiving ISR clears the register.
        let mut irqs = Vec::new();
        for (n, &v) in self.from_cpu.iter().enumerate() {
            if v != 0 {
                irqs.push(FROM_CPU_SOURCE_BASE + n as u32);
            }
        }
        PeripheralTickResult {
            explicit_irqs: if irqs.is_empty() { None } else { Some(irqs) },
            ..PeripheralTickResult::default()
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_asserts_nothing() {
        let mut ipi = Esp32s3CrossCoreIpi::new();
        assert!(ipi.tick().explicit_irqs.is_none());
    }

    #[test]
    fn from_cpu0_asserts_source_79_until_cleared() {
        let mut ipi = Esp32s3CrossCoreIpi::new();
        // Trigger core-0 doorbell (offset 0x00 within the block).
        ipi.write_u32(0x00, 1).unwrap();
        assert_eq!(ipi.read_u32(0x00).unwrap(), 1, "latch round-trips");
        // Level source re-asserts every tick.
        assert_eq!(ipi.tick().explicit_irqs, Some(vec![79]));
        assert_eq!(ipi.tick().explicit_irqs, Some(vec![79]));
        // ISR clears the doorbell → source deasserts next tick.
        ipi.write_u32(0x00, 0).unwrap();
        assert!(ipi.tick().explicit_irqs.is_none());
    }

    #[test]
    fn from_cpu1_asserts_source_80() {
        let mut ipi = Esp32s3CrossCoreIpi::new();
        // FROM_CPU_1 is the second word (offset 0x04).
        ipi.write_u32(0x04, 1).unwrap();
        assert_eq!(ipi.tick().explicit_irqs, Some(vec![80]));
    }

    #[test]
    fn byte_writes_compose_a_word() {
        let mut ipi = Esp32s3CrossCoreIpi::new();
        ipi.write(0x00, 0x01).unwrap();
        assert_eq!(ipi.tick().explicit_irqs, Some(vec![79]));
    }
}

#[cfg(test)]
mod integration {
    use super::*;
    use crate::bus::SystemBus;
    use crate::system::xtensa::{configure_xtensa_esp32s3, Esp32s3Opts};
    use crate::Bus;

    #[test]
    fn from_cpu_tick_sets_pending_when_mapped() {
        let mut bus = SystemBus::new();
        let _ = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
        assert!(bus.esp32s3_irq_routing, "S3 irq routing should be on");
        // Program intmatrix: source 79 -> cpu0 irq 1, source 80 -> cpu1 irq 1
        // CORE0 map @ 0x600C2000 + 4*src
        // CORE1 map @ 0x600C2000 + 0x800 + 4*src
        bus.write_u32(0x600C_2000 + 79 * 4, 1).unwrap();
        bus.write_u32(0x600C_2000 + 0x800 + 80 * 4, 1).unwrap();
        // Ring doorbells
        bus.write_u32(0x600C_0030, 1).unwrap();
        bus.write_u32(0x600C_0034, 1).unwrap();
        // Tick peripherals
        let mut irqs = Vec::new();
        let mut costs = Vec::new();
        bus.tick_peripherals_fully_into(&mut irqs, &mut costs);
        let p0 = bus.pending_cpu_irqs(0);
        let p1 = bus.pending_cpu_irqs(1);
        eprintln!("pending after tick: p0={p0:#010x} p1={p1:#010x} raw_irqs={irqs:?}");
        assert_ne!(p0, 0, "core0 should see FROM_CPU_0 source 79");
        assert_ne!(p1, 0, "core1 should see FROM_CPU_1 source 80");
    }
}
