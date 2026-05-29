// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::SimResult;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// Bundle of CortexM-shared SCB fields. Passed to `Scb::with_shared`
/// when the CPU and SCB are wired by `configure_cortex_m`.
pub struct SharedScbState {
    pub vtor: Arc<AtomicU32>,
    pub vectactive: Arc<AtomicU32>,
    pub shpr1: Arc<AtomicU32>,
    pub shpr2: Arc<AtomicU32>,
    pub shpr3: Arc<AtomicU32>,
}

/// System Control Block (SCB)
#[derive(Debug, serde::Serialize)]
pub struct Scb {
    pub cpuid: u32,
    pub icsr: u32,
    #[serde(skip)]
    pub vtor: Arc<AtomicU32>, // Shared with CPU
    #[serde(skip)]
    /// Shared with CPU: bits 0..8 of ICSR.VECTACTIVE. Read-only mirror
    /// of the CPU's currently-active exception number. cortex-m-rt's
    /// DefaultHandler reads ICSR to identify which IRQ fired, so this
    /// must be live or the handler can't dispatch correctly.
    pub vectactive: Arc<AtomicU32>,
    pub aircr: u32,
    pub scr: u32,
    pub ccr: u32,
    #[serde(skip)]
    /// SHPR1 (offset 0x18) holds priorities for MemManage(4), BusFault(5),
    /// UsageFault(6). Shared with CortexM so its exception-dispatch path
    /// can compute ARM-priority-correct preemption decisions.
    pub shpr1: Arc<AtomicU32>,
    #[serde(skip)]
    /// SHPR2 (offset 0x1C) holds priority for SVCall(11) in byte 3.
    pub shpr2: Arc<AtomicU32>,
    #[serde(skip)]
    /// SHPR3 (offset 0x20) holds priorities for PendSV(14) in byte 2 and
    /// SysTick(15) in byte 3. FreeRTOS configures PendSV to lowest
    /// priority (0xFF) so the context-switch handler only runs when no
    /// other interrupt is active — that's the load-bearing semantics
    /// for `loopTask` to ever get CPU time.
    pub shpr3: Arc<AtomicU32>,
    /// PendSV exception pend bit. Set by an ICSR.PENDSVSET write
    /// (bit 28); drained into the CPU's pending_exceptions via tick().
    pub pendsv_pending: bool,
    /// SysTick exception pend bit (ICSR.PENDSTSET=bit 26).
    pub systick_pending: bool,
    /// NMI pend bit (ICSR.NMIPENDSET=bit 31).
    pub nmi_pending: bool,
}

impl Scb {
    pub fn new(vtor: Arc<AtomicU32>) -> Self {
        Self::with_shared(SharedScbState {
            vtor,
            vectactive: Arc::new(AtomicU32::new(0)),
            shpr1: Arc::new(AtomicU32::new(0)),
            shpr2: Arc::new(AtomicU32::new(0)),
            shpr3: Arc::new(AtomicU32::new(0)),
        })
    }

    pub fn with_vectactive(vtor: Arc<AtomicU32>, vectactive: Arc<AtomicU32>) -> Self {
        Self::with_shared(SharedScbState {
            vtor,
            vectactive,
            shpr1: Arc::new(AtomicU32::new(0)),
            shpr2: Arc::new(AtomicU32::new(0)),
            shpr3: Arc::new(AtomicU32::new(0)),
        })
    }

    pub fn with_shared(s: SharedScbState) -> Self {
        Self {
            cpuid: 0x410F_C241,
            icsr: 0,
            vtor: s.vtor,
            vectactive: s.vectactive,
            aircr: 0,
            scr: 0,
            ccr: 0,
            shpr1: s.shpr1,
            shpr2: s.shpr2,
            shpr3: s.shpr3,
            pendsv_pending: false,
            systick_pending: false,
            nmi_pending: false,
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cpuid,
            0x04 => {
                // ICSR: only VECTACTIVE [8:0] is modeled live. The rest
                // (VECTPENDING [22:12], ISRPREEMPT [23], PENDSV [28],
                // NMIPENDSET [31] etc.) come from the stored icsr.
                (self.icsr & !0x1FF) | (self.vectactive.load(Ordering::Relaxed) & 0x1FF)
            }
            0x08 => self.vtor.load(Ordering::Relaxed),
            0x0C => self.aircr,
            0x10 => self.scr,
            0x14 => self.ccr,
            0x18 => self.shpr1.load(Ordering::Relaxed),
            0x1C => self.shpr2.load(Ordering::Relaxed),
            0x20 => self.shpr3.load(Ordering::Relaxed),
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x04 => {
                // ICSR side effects (ARMv7-M ARM B3.2.4):
                //   bit 31 NMIPENDSET — pend NMI (2)
                //   bit 28 PENDSVSET  — pend PendSV (14); needed for
                //                       FreeRTOS context switches.
                //   bit 27 PENDSVCLR  — clear PendSV pending
                //   bit 26 PENDSTSET  — pend SysTick (15)
                //   bit 25 PENDSTCLR  — clear SysTick pending
                // tick() drains these into the CPU's pending_exceptions
                // via the standard system_exception result field.
                if value & (1 << 31) != 0 {
                    self.nmi_pending = true;
                }
                if value & (1 << 28) != 0 {
                    self.pendsv_pending = true;
                }
                if value & (1 << 27) != 0 {
                    self.pendsv_pending = false;
                }
                if value & (1 << 26) != 0 {
                    self.systick_pending = true;
                }
                if value & (1 << 25) != 0 {
                    self.systick_pending = false;
                }
                self.icsr = value;
            }
            0x08 => self.vtor.store(value, Ordering::Relaxed),
            0x0C => self.aircr = value,
            0x10 => self.scr = value,
            0x14 => self.ccr = value,
            0x18 => self.shpr1.store(value, Ordering::Relaxed),
            0x1C => self.shpr2.store(value, Ordering::Relaxed),
            0x20 => self.shpr3.store(value, Ordering::Relaxed),
            _ => {}
        }
    }
}

impl crate::Peripheral for Scb {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = self.read_reg(reg_offset);
        Ok(((reg_val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let mut reg_val = self.read_reg(reg_offset);

        let mask = 0xFF << (byte_offset * 8);
        reg_val &= !mask;
        reg_val |= (value as u32) << (byte_offset * 8);

        self.write_reg(reg_offset, reg_val);
        Ok(())
    }

    fn tick(&mut self) -> crate::PeripheralTickResult {
        // Drain pending system-exception bits set by ICSR writes. NMI
        // takes priority over SysTick over PendSV when multiple are
        // pending simultaneously (per ARMv7-M priority table).
        if self.nmi_pending {
            self.nmi_pending = false;
            return crate::PeripheralTickResult {
                system_exception: Some(2),
                cycles: 1,
                ..Default::default()
            };
        }
        if self.systick_pending {
            self.systick_pending = false;
            return crate::PeripheralTickResult {
                system_exception: Some(15),
                cycles: 1,
                ..Default::default()
            };
        }
        if self.pendsv_pending {
            self.pendsv_pending = false;
            return crate::PeripheralTickResult {
                system_exception: Some(14),
                cycles: 1,
                ..Default::default()
            };
        }
        crate::PeripheralTickResult::default()
    }

    fn snapshot(&self) -> serde_json::Value {
        let mut value = serde_json::to_value(self).unwrap_or(serde_json::Value::Null);
        // Inject VTOR value manually since we skip the Arc
        if let Some(obj) = value.as_object_mut() {
            obj.insert(
                "vtor".to_string(),
                serde_json::Value::Number(self.vtor.load(Ordering::Relaxed).into()),
            );
        }
        value
    }
}
