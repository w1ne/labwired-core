// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::SimResult;

/// Mocked SysTick Timer peripheral
/// Standard address: 0xE000_E010
#[derive(Debug, Default, serde::Serialize)]
pub struct Systick {
    /// SYST_CSR config bits (ENABLE/TICKINT/CLKSOURCE). COUNTFLAG (bit 16) is
    /// NOT stored here — it is held in `countflag` so a read of CSR can clear
    /// it (the ARMv7-M "reads clear COUNTFLAG" rule, which `cortex_m_systick`
    /// relies on; see `read_u32`).
    csr: u32,
    rvr: u32,
    cvr: u32,
    calib: u32,
    /// SYST_CSR.COUNTFLAG. Set when the counter wraps to 0; cleared on a read
    /// of SYST_CSR or a write of SYST_CVR. `Cell` so the `&self` read path can
    /// clear it.
    countflag: std::cell::Cell<bool>,
}

impl Systick {
    pub fn new() -> Self {
        Self {
            csr: 0,
            rvr: 0,
            cvr: 0,
            calib: 0x4000_0000, // No reference clock, no skew
            countflag: std::cell::Cell::new(false),
        }
    }

    /// CALIB is implementation-defined per chip (TENMS/SKEW/NOREF). The chip
    /// yaml can supply the silicon value via `config: { calib: ... }`.
    pub fn with_calib(calib: u32) -> Self {
        Self {
            calib,
            ..Self::new()
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            // COUNTFLAG (bit 16) folded in from `countflag`. Byte reads do not
            // clear it; the word read (`read_u32`) does, matching how the
            // driver accesses CSR.
            0x00 => self.csr | if self.countflag.get() { 0x1_0000 } else { 0 },
            0x04 => self.rvr,
            0x08 => self.cvr,
            0x0C => self.calib,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => {
                self.csr = value & 0x7;
                if std::env::var("LABWIRED_TRACE_SYSTICK").is_ok() {
                    eprintln!(
                        "SYSTICK CSR <- 0x{:08X} (enable={} tickint={} clksrc={})",
                        value,
                        value & 1,
                        (value >> 1) & 1,
                        (value >> 2) & 1
                    );
                }
            }
            0x04 => {
                self.rvr = value & 0x00FF_FFFF;
                if std::env::var("LABWIRED_TRACE_SYSTICK").is_ok() {
                    eprintln!(
                        "SYSTICK RVR <- 0x{:08X} ({})",
                        value & 0x00FF_FFFF,
                        value & 0x00FF_FFFF
                    );
                }
            }
            0x08 => {
                // Writing SYST_CVR clears the counter and COUNTFLAG.
                self.cvr = 0;
                self.countflag.set(false);
            }
            _ => {}
        }
    }
}

impl crate::Peripheral for Systick {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = self.read_reg(reg_offset);
        Ok(((reg_val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let val = self.read_reg(offset);
        // ARMv7-M: a read of SYST_CSR returns COUNTFLAG and then clears it.
        // `cortex_m_systick`'s elapsed() depends on this to detect a wrap, so
        // it must happen on the word read the driver issues.
        if offset == 0x00 {
            self.countflag.set(false);
        }
        Ok(val)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let mut reg_val = self.read_reg(reg_offset);

        // Modify byte
        let mask = 0xFF << (byte_offset * 8);
        reg_val &= !mask;
        reg_val |= (value as u32) << (byte_offset * 8);

        self.write_reg(reg_offset, reg_val);
        Ok(())
    }

    fn tick(&mut self) -> crate::PeripheralTickResult {
        if (self.csr & 0x1) == 0 {
            return crate::PeripheralTickResult {
                cycles: 0,
                ..Default::default()
            };
        }

        // ARMv7-M SysTick is EDGE-triggered: COUNTFLAG/interrupt are raised only
        // when the counter COUNTS DOWN to zero, never when it merely holds zero.
        // A zero counter (after reset, or after software writes SYST_CVR — which
        // hardware clears to 0) reloads from RVR on the next clock WITHOUT firing.
        // Modelling "cvr == 0 ⇒ fire" instead would make every `SYST_CVR <- 0`
        // the tickless-idle path issues each ISR re-pend SysTick immediately, so
        // the handler is re-entered before the interrupted thread can run.
        if self.cvr == 0 {
            // Reload only; the previous wrap already fired (or this is the
            // initial/software-cleared zero).
            self.cvr = self.rvr;
            return crate::PeripheralTickResult {
                irq: false,
                cycles: 1,
                ..Default::default()
            };
        }

        self.cvr -= 1;
        if self.cvr != 0 {
            return crate::PeripheralTickResult {
                irq: false,
                cycles: 1,
                ..Default::default()
            };
        }

        // Counter just transitioned to zero: set COUNTFLAG and raise the tick.
        self.countflag.set(true);
        // SysTick raises system exception 15 — NOT an NVIC IRQ. The bus
        // dispatches `system_exception` directly to the CPU's pending_exceptions
        // bitmap, bypassing NVIC ISER/ISPR. (Routing through NVIC would interpret
        // 15 as NVIC IRQ 15 = exception 31, which has no vector in standard
        // STM32 firmware.)
        let fire = (self.csr & 0x2) != 0;
        crate::PeripheralTickResult {
            irq: false,
            cycles: 1,
            dma_requests: None,
            system_exception: if fire { Some(15) } else { None },
            ..Default::default()
        }
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    /// Enable SysTick with the given reload, leaving the counter cleared to 0
    /// (as hardware does on a SYST_CVR write).
    fn armed(reload: u8) -> Systick {
        let mut st = Systick::new();
        st.write(0x04, reload).unwrap(); // RVR
        st.write(0x08, 0x00).unwrap(); // CVR write → clears counter to 0
        st.write(0x00, 0x07).unwrap(); // CSR: ENABLE | TICKINT | CLKSOURCE
        st
    }

    /// Regression: a counter sitting at zero (freshly reloaded, or just cleared
    /// by a software SYST_CVR write) must RELOAD without firing. Only a count-
    /// down transition to zero raises the tick. Modelling "cvr == 0 ⇒ fire"
    /// makes the tickless-idle SYST_CVR write re-pend SysTick every step, so the
    /// ISR is re-entered before the interrupted thread can run (STM32 + Zephyr
    /// never reached main).
    #[test]
    fn fires_only_on_countdown_to_zero_not_on_loaded_zero() {
        let mut st = armed(4);

        // tick 1: counter is 0 (software-cleared) → reload to RVR, NO fire.
        assert_eq!(
            st.tick().system_exception,
            None,
            "reload tick must not fire"
        );
        // ticks 2..=4: 4→3→2→1, no fire yet.
        for _ in 0..3 {
            assert_eq!(st.tick().system_exception, None);
        }
        // tick 5: 1→0, the count-down edge → fire SysTick (exception 15).
        assert_eq!(
            st.tick().system_exception,
            Some(15),
            "count-down to zero must raise SysTick"
        );
        // tick 6: counter is 0 again → reload, NO fire.
        assert_eq!(
            st.tick().system_exception,
            None,
            "post-fire reload must not fire"
        );
    }

    /// A software write to SYST_CVR clears the counter; the immediately following
    /// tick must reload from RVR rather than fire (the exact path the Zephyr
    /// tickless idle exercises on every ISR).
    #[test]
    fn software_cvr_write_does_not_fire() {
        let mut st = armed(3);
        // Let it run partway down.
        st.tick(); // reload to 3
        st.tick(); // 3→2
                   // Software clears the counter.
        st.write(0x08, 0x00).unwrap();
        // Next tick reloads, does not fire.
        assert_eq!(st.tick().system_exception, None);
    }

    /// COUNTFLAG (SYST_CSR bit 16) reads as set after a wrap and is cleared by
    /// the word read, per ARMv7-M. `cortex_m_systick`'s elapsed() relies on this.
    #[test]
    fn countflag_sets_on_wrap_and_clears_on_read() {
        let mut st = armed(2);
        st.tick(); // reload to 2
        st.tick(); // 2→1
        assert_eq!(st.tick().system_exception, Some(15)); // 1→0, fire

        let csr = st.read_u32(0x00).unwrap();
        assert_eq!(csr & 0x1_0000, 0x1_0000, "COUNTFLAG set after wrap");
        let csr2 = st.read_u32(0x00).unwrap();
        assert_eq!(csr2 & 0x1_0000, 0, "COUNTFLAG cleared by the read");
    }

    /// A disabled SysTick (ENABLE=0) never ticks down or fires.
    #[test]
    fn disabled_systick_is_inert() {
        let mut st = Systick::new();
        st.write(0x04, 1).unwrap();
        st.write(0x08, 0x00).unwrap();
        // ENABLE not set.
        for _ in 0..10 {
            let r = st.tick();
            assert_eq!(r.system_exception, None);
            assert_eq!(r.cycles, 0);
        }
    }
}
