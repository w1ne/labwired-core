// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::{CycleClock, SimResult};
use std::cell::Cell;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// Event token for the ICSR pend-drain chain (the SCB schedules exactly one
/// kind of event, so a constant token suffices — no arming sequence needed:
/// claims are idempotent and a duplicate chain dies on an empty latch set).
const SCB_PEND_DRAIN_TOKEN: u32 = 0;

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
    /// Number of MPU regions reported in MPU_TYPE.DREGION (bits [15:8]).
    /// Confirmed by a live SWD read of the nRF52840 on 2026-06-23 (ST-LINK V2,
    /// FICR.INFO.PART=0x00052840): MPU_TYPE reads 0x0000_0800, i.e. DREGION = 8,
    /// matching the Cortex-M4F datasheet. CTRL/RNR/RBAR/RASR all read 0 at reset.
    /// 0 here means "no MPU". Region programming is accepted but not enforced yet.
    pub mpu_dregion: u32,
    /// MPU_CTRL (0xE000ED94): ENABLE/HFNMIENA/PRIVDEFENA. Stored, not enforced.
    pub mpu_ctrl: u32,
    /// MPU_RNR (0xE000ED98): selected region number.
    pub mpu_rnr: u32,
    /// MPU_RBAR (0xE000ED9C): region base address register.
    pub mpu_rbar: u32,
    /// MPU_RASR (0xE000EDA0): region attribute and size register.
    pub mpu_rasr: u32,
    /// MPU_MAIR0/MAIR1 (0xE000EDC0/0xC4): ARMv8-M memory attribute indirection
    /// registers. Present on Cortex-M33 parts (the M23/M33 MPU replaces the
    /// v7-M RASR attribute encoding with an 8-entry MAIR table). Zephyr's
    /// `z_arm_mpu_init` writes these on M33 targets; stored so the access
    /// round-trips, not enforced (attributes have no effect in the model).
    pub mpu_mair0: u32,
    pub mpu_mair1: u32,
    /// Set when firmware writes AIRCR with the correct VECTKEY and SYSRESETREQ.
    /// Drained by the machine reset routing via drain_reset_request().
    #[serde(skip)]
    pending_reset: Cell<bool>,
    /// Walk-free plan batch B1: bus cycle clock, attached by the registration
    /// choke (`configure_cortex_m`). Used purely as the "machine-driven bus"
    /// marker that flips `uses_scheduler()` — the SCB has no time-derived
    /// state, so the clock's value is never read. `None` (hand-built test
    /// buses / feature off) keeps the legacy per-tick drain.
    #[serde(skip)]
    clock: Option<CycleClock>,
    /// True while an ICSR pend-drain event chain is in flight. Ensures exactly
    /// ONE chain exists no matter how many ICSR writes land before it drains,
    /// preserving the legacy walk's one-exception-per-cycle pacing (NMI, then
    /// SysTick, then PendSV on consecutive cycles).
    #[serde(skip)]
    drain_chain_armed: bool,
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
            // 8-region MPU. Silicon-confirmed on the nRF52840 (2026-06-23 SWD
            // read: MPU_TYPE=0x0000_0800, DREGION=8). See the field doc above.
            mpu_dregion: 8,
            mpu_ctrl: 0,
            mpu_rnr: 0,
            mpu_rbar: 0,
            mpu_rasr: 0,
            mpu_mair0: 0,
            mpu_mair1: 0,
            pending_reset: Cell::new(false),
            clock: None,
            drain_chain_armed: false,
        }
    }

    /// True when the event scheduler owns the ICSR pend-drain (feature on AND
    /// the bus attached its cycle clock at registration). The single predicate
    /// both `uses_scheduler()` and the legacy-tick guard branch on, so the two
    /// drive modes can never mix.
    #[inline]
    fn scheduler_mode(&self) -> bool {
        cfg!(feature = "event-scheduler") && self.clock.is_some()
    }

    /// Test/differential knob: detach the cycle clock, pinning the model to
    /// the legacy walk path (`uses_scheduler() == false`). Lets the
    /// walk-on-vs-scheduler differential gates build the reference lane from
    /// the same bus assembly.
    pub fn force_legacy_walk(&mut self) {
        self.clock = None;
        self.drain_chain_armed = false;
    }

    /// True while any ICSR-latched system exception awaits delivery.
    #[inline]
    fn any_pend_latched(&self) -> bool {
        self.nmi_pending || self.systick_pending || self.pendsv_pending
    }

    /// Drain ONE latched pend in architectural priority order (NMI > SysTick >
    /// PendSV — the ARMv7-M priority table), returning its exception number.
    /// Shared by the legacy `tick()` and the scheduler `on_event` so both
    /// drive modes deliver identical sequences.
    fn drain_one_pend(&mut self) -> Option<u32> {
        if self.nmi_pending {
            self.nmi_pending = false;
            Some(2)
        } else if self.systick_pending {
            self.systick_pending = false;
            Some(15)
        } else if self.pendsv_pending {
            self.pendsv_pending = false;
            Some(14)
        } else {
            None
        }
    }

    /// Returns true once if a SYSRESETREQ was latched, then clears the latch.
    pub fn drain_reset_request(&self) -> bool {
        self.pending_reset.replace(false)
    }

    /// Write a 32-bit value to an SCB register at the given word-aligned offset.
    /// Only compiled in test builds; production MMIO goes through `Peripheral::write`.
    #[cfg(test)]
    pub fn write_register(&mut self, offset: u64, value: u32) {
        self.write_reg(offset, value);
    }

    /// Read a 32-bit SCB/SCS register at the given word-aligned offset.
    /// Test-only; production MMIO goes through `Peripheral::read`.
    #[cfg(test)]
    pub fn read_register(&self, offset: u64) -> u32 {
        self.read_reg(offset)
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cpuid,
            0x04 => {
                // ICSR. VECTACTIVE [8:0] is live; the pend bits PENDSV [28],
                // PENDSTSET [26] and NMIPENDSET [31] read back live pending
                // state. The write-only SET/CLR action bits never read back
                // (ARMv7-M ARM B3.2.4) — they are masked out of the stored
                // value on write, so PENDSVCLR/PENDSTCLR always read 0.
                let mut v =
                    (self.icsr & !0x1FF) | (self.vectactive.load(Ordering::Relaxed) & 0x1FF);
                if self.pendsv_pending {
                    v |= 1 << 28;
                }
                if self.systick_pending {
                    v |= 1 << 26;
                }
                if self.nmi_pending {
                    v |= 1 << 31;
                }
                v
            }
            0x08 => self.vtor.load(Ordering::Relaxed),
            0x0C => self.aircr,
            0x10 => self.scr,
            0x14 => self.ccr,
            0x18 => self.shpr1.load(Ordering::Relaxed),
            0x1C => self.shpr2.load(Ordering::Relaxed),
            0x20 => self.shpr3.load(Ordering::Relaxed),
            // MPU (ARMv7-M, 0xE000ED90..0xE000EDA0). TYPE is read-only:
            // SEPARATE=0 (unified), DREGION=[15:8], IREGION=0.
            0x90 => (self.mpu_dregion & 0xFF) << 8,
            0x94 => self.mpu_ctrl,
            0x98 => self.mpu_rnr,
            0x9C => self.mpu_rbar,
            0xA0 => self.mpu_rasr,
            // ARMv8-M MPU memory attribute indirection registers (Cortex-M33).
            0xC0 => self.mpu_mair0,
            0xC4 => self.mpu_mair1,
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
                // The SET/CLR action bits are write-only / self-clearing: never
                // persist them. Zephyr's arch_swap re-pends with a read-modify-
                // write (`ldr ICSR; orr #PENDSVSET; str ICSR`); storing the bits
                // would read back a stale PENDSVCLR, and its side effect above
                // would then cancel the fresh PENDSVSET — leaving PendSV unpended,
                // so a self-aborting thread's context switch never fires.
                const ICSR_ACTION_BITS: u32 =
                    (1 << 31) | (1 << 28) | (1 << 27) | (1 << 26) | (1 << 25);
                self.icsr = value & !ICSR_ACTION_BITS;
            }
            0x08 => self.vtor.store(value, Ordering::Relaxed),
            0x0C => {
                if (value >> 16) == 0x05FA && value & (1 << 2) != 0 {
                    self.pending_reset.set(true);
                }
                // Store masked: VECTKEY field reads back as 0 (matches silicon).
                self.aircr = value & 0x0000_FFFF;
            }
            0x10 => self.scr = value,
            0x14 => self.ccr = value,
            0x18 => self.shpr1.store(value, Ordering::Relaxed),
            0x1C => self.shpr2.store(value, Ordering::Relaxed),
            0x20 => self.shpr3.store(value, Ordering::Relaxed),
            // MPU region programming. Stored so reads round-trip and
            // z_arm_mpu_init completes; access enforcement is not modeled yet.
            // 0x90 (TYPE) is read-only — writes are ignored.
            0x94 => self.mpu_ctrl = value,
            0x98 => self.mpu_rnr = value,
            0x9C => self.mpu_rbar = value,
            0xA0 => self.mpu_rasr = value,
            // ARMv8-M MPU MAIR0/MAIR1 (Cortex-M33). Stored for round-trip; the
            // attribute encodings have no effect since access is not enforced.
            0xC0 => self.mpu_mair0 = value,
            0xC4 => self.mpu_mair1 = value,
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

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        // Firmware writes SCB registers with a full-word store (the CPU's
        // STR lands on the bus's word path, which calls this). AIRCR is an
        // action-on-write register whose VECTKEY (bits 31:16) reads back as
        // 0 — so the default byte-by-byte decomposition would never see the
        // VECTKEY and SYSRESETREQ together. Dispatch the coherent 32-bit
        // value straight to `write_reg` so the reset latch (and the ICSR
        // pend bits) react to the value the firmware actually wrote.
        self.write_reg(offset & !3, value);
        Ok(())
    }

    fn tick(&mut self) -> crate::PeripheralTickResult {
        // Never runs in scheduler mode (the walk skips `uses_scheduler()`
        // peripherals; the guard keeps a stray direct call from racing the
        // event chain). Legacy mode: drain ONE pending system-exception bit
        // set by ICSR writes per tick, NMI > SysTick > PendSV.
        //
        // B1 tick-cost normalization: the drain used to charge `cycles: 1`
        // into the tick-cost channel, inflating `total_cycles` by one per
        // drained pend — a sim artifact (pending a system exception consumes
        // no core cycles on silicon) incompatible with byte-identity between
        // the walk and the scheduler path. Both modes now charge zero.
        if self.scheduler_mode() {
            return crate::PeripheralTickResult::default();
        }
        match self.drain_one_pend() {
            Some(exc) => crate::PeripheralTickResult {
                system_exception: Some(exc),
                ..Default::default()
            },
            None => crate::PeripheralTickResult::default(),
        }
    }

    fn uses_scheduler(&self) -> bool {
        // True once the bus attached its cycle clock (event-scheduler builds):
        // ICSR pends are delivered by write-armed delay-0 events, so the
        // per-cycle walk has nothing left to do here. Without a clock
        // (feature off / hand-built buses) stay on the legacy walk.
        self.scheduler_mode()
    }

    fn take_scheduled_events(&mut self) -> Vec<(u64, u32)> {
        // Called by the bus after every MMIO write to this peripheral: if the
        // write latched a pend (ICSR SET bits) and no drain chain is in
        // flight, arm one at delay 0 — `collect_scheduled_events` converts it
        // to `current_cycle + 1`, the exact cycle the legacy walk would have
        // drained the first pend at (interval 1, run path).
        if !self.scheduler_mode() || !self.any_pend_latched() || self.drain_chain_armed {
            return Vec::new();
        }
        self.drain_chain_armed = true;
        vec![(0, SCB_PEND_DRAIN_TOKEN)]
    }

    fn on_event(
        &mut self,
        _event_token: u32,
        _sched: &mut crate::sched::EventScheduler,
        _bus: &mut dyn crate::Bus,
    ) -> crate::sched::EventResult {
        if !self.scheduler_mode() {
            return crate::sched::EventResult::default();
        }
        // Drain ONE pend per event, rescheduling one cycle later while more
        // remain — replicating the legacy walk's one-exception-per-cycle
        // pacing (NMI, then SysTick, then PendSV on consecutive cycles).
        let exc = self.drain_one_pend();
        self.drain_chain_armed = self.any_pend_latched();
        crate::sched::EventResult {
            system_exception: exc,
            reschedule_delay: if self.drain_chain_armed {
                Some(1)
            } else {
                None
            },
            ..Default::default()
        }
    }

    fn attach_cycle_clock(&mut self, clock: CycleClock) {
        self.clock = Some(clock);
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    #[test]
    fn aircr_sysresetreq_with_vectkey_latches_reset() {
        let mut scb = Scb::new(Arc::new(AtomicU32::new(0)));
        scb.write_register(0x0C, (0x05FA << 16) | (1 << 2)); // VECTKEY + SYSRESETREQ
        assert!(scb.drain_reset_request());
        assert!(!scb.drain_reset_request()); // latch cleared
    }

    #[test]
    fn aircr_without_vectkey_does_not_reset() {
        let mut scb = Scb::new(Arc::new(AtomicU32::new(0)));
        scb.write_register(0x0C, 1 << 2); // SYSRESETREQ but no key
        assert!(!scb.drain_reset_request());
    }

    #[test]
    fn mpu_type_reports_eight_regions() {
        // nRF52840 is a Cortex-M4F with an 8-region MPU. z_arm_mpu_init reads
        // MPU_TYPE.DREGION (bits [15:8]) at offset 0x90 and asserts if it is
        // smaller than the configured region count; an unmodeled MPU read 0 and
        // hung the boot. TYPE = DREGION << 8; SEPARATE/IREGION are 0 on ARMv7-M.
        //
        // This expectation is silicon-locked: a 2026-06-23 SWD read of the real
        // nRF52840 (FICR.INFO.PART=0x00052840) returned MPU_TYPE=0x0000_0800.
        // Changing the model away from DREGION=8 must fail here — the value is
        // measured silicon, not a guess.
        let scb = Scb::new(Arc::new(AtomicU32::new(0)));
        let mpu_type = scb.read_register(0x90);
        assert_eq!((mpu_type >> 8) & 0xFF, 8, "DREGION");
        assert_eq!(mpu_type & 0x1, 0, "SEPARATE (unified)");
        assert_eq!((mpu_type >> 16) & 0xFF, 0, "IREGION");
    }

    #[test]
    fn mpu_ctrl_rnr_rbar_rasr_are_read_write() {
        // The region-programming registers must round-trip so z_arm_mpu_init can
        // configure regions and enable the MPU. Enforcement is not modeled yet.
        let mut scb = Scb::new(Arc::new(AtomicU32::new(0)));
        scb.write_register(0x94, 0x5); // CTRL: ENABLE | PRIVDEFENA
        scb.write_register(0x98, 0x3); // RNR: region 3
        scb.write_register(0x9C, 0x2000_0013); // RBAR
        scb.write_register(0xA0, 0x0300_0027); // RASR
        assert_eq!(scb.read_register(0x94), 0x5, "CTRL");
        assert_eq!(scb.read_register(0x98), 0x3, "RNR");
        assert_eq!(scb.read_register(0x9C), 0x2000_0013, "RBAR");
        assert_eq!(scb.read_register(0xA0), 0x0300_0027, "RASR");
    }

    #[test]
    fn mpu_mair_round_trips_on_armv8m() {
        // Cortex-M33 (ARMv8-M) z_arm_mpu_init writes the attribute table to
        // MAIR0/MAIR1 at SCS offset 0xC0/0xC4 — just past the v7-M MPU block.
        // The SCS window now covers them (system::cortex_m sizes the SCB region
        // to 0xC8); an unmapped store here previously faulted the bus on every
        // M33 boot (observed bringing up Zephyr on the nRF5340 application core).
        let mut scb = Scb::new(Arc::new(AtomicU32::new(0)));
        // Reset value is 0 (no attributes programmed).
        assert_eq!(scb.read_register(0xC0), 0, "MAIR0 reset");
        assert_eq!(scb.read_register(0xC4), 0, "MAIR1 reset");
        scb.write_register(0xC0, 0x0000_00AA);
        scb.write_register(0xC4, 0x0444_0000);
        assert_eq!(scb.read_register(0xC0), 0x0000_00AA, "MAIR0");
        assert_eq!(scb.read_register(0xC4), 0x0444_0000, "MAIR1");
    }

    #[test]
    fn icsr_pendsvclr_is_write_only() {
        // PENDSVCLR [27] is a write-only action bit (ARMv7-M ARM B3.2.4); it
        // must always read back 0, never the last written value.
        let mut scb = Scb::new(Arc::new(AtomicU32::new(0)));
        scb.write_register(0x04, 1 << 27); // PENDSVCLR
        assert_eq!(scb.read_register(0x04) & (1 << 27), 0, "PENDSVCLR reads 0");
    }

    #[test]
    fn icsr_pendsvset_reads_live_pending() {
        // PENDSV [28] reads back the live pending state: set after PENDSVSET,
        // cleared once the exception is serviced — not the written action bit.
        let mut scb = Scb::new(Arc::new(AtomicU32::new(0)));
        scb.write_register(0x04, 1 << 28); // PENDSVSET
        assert_eq!(scb.read_register(0x04) & (1 << 28), 1 << 28, "pending");
        let _ = scb.tick(); // service PendSV
        assert_eq!(scb.read_register(0x04) & (1 << 28), 0, "drained reads 0");
    }

    #[test]
    fn icsr_stale_pendsvclr_does_not_poison_later_pend() {
        // Regression for the Zephyr sched.c:493 abort. arch_swap re-pends with
        // `ldr ICSR; orr #PENDSVSET; str ICSR`. If a PENDSVCLR written earlier
        // read back stale, the OR-and-store would carry it along and the
        // PENDSVCLR side effect would cancel the fresh PENDSVSET — no context
        // switch, and z_swap returns into the dying thread.
        let mut scb = Scb::new(Arc::new(AtomicU32::new(0)));
        scb.write_register(0x04, 1 << 28); // start swap: pend PendSV
        let _ = scb.tick(); // serviced
        scb.write_register(0x04, 1 << 27); // kernel clears: PENDSVCLR
        let v = scb.read_register(0x04); // RMW read
        scb.write_register(0x04, v | (1 << 28)); // orr PENDSVSET; str
        assert!(
            scb.pendsv_pending,
            "PendSV pending; stale CLR must not survive"
        );
    }

    #[test]
    fn icsr_nmi_systick_pend_read_live_clr_reads_zero() {
        let mut scb = Scb::new(Arc::new(AtomicU32::new(0)));
        scb.write_register(0x04, 1 << 31); // NMIPENDSET
        assert_eq!(scb.read_register(0x04) & (1 << 31), 1 << 31, "NMI pending");
        scb.write_register(0x04, 1 << 26); // PENDSTSET
        assert_eq!(
            scb.read_register(0x04) & (1 << 26),
            1 << 26,
            "SysTick pending"
        );
        scb.write_register(0x04, 1 << 25); // PENDSTCLR
        assert_eq!(scb.read_register(0x04) & (1 << 25), 0, "PENDSTCLR reads 0");
        assert_eq!(scb.read_register(0x04) & (1 << 26), 0, "SysTick cleared");
    }
}
