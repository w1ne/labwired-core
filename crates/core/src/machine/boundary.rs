//! Owns CPU execution and post-execution lifecycle commits.

use crate::{Cpu, Machine, SimResult};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExecutionMode {
    SingleDirect,
    RunBatch,
    RunDual,
}

#[derive(Clone, Copy)]
pub(crate) struct CoreProgress {
    pub primary_steps: u32,
    pub secondary_steps: u32,
}

impl<C: Cpu> Machine<C> {
    pub(crate) fn execute_cpu_window(
        &mut self,
        mode: ExecutionMode,
        count: u32,
    ) -> SimResult<CoreProgress> {
        match mode {
            ExecutionMode::SingleDirect | ExecutionMode::RunDual => {
                debug_assert_eq!(count, 1);
                self.total_cycles += 1;
                self.bus.set_current_cycle(self.total_cycles);
                self.bus.bus_trace.set_cycle(self.total_cycles);
                if self.logic_capture.push_active() {
                    self.bus.logic_tap.set_clock(self.total_cycles);
                }
            }
            ExecutionMode::RunBatch => {
                self.bus.set_current_cycle(self.total_cycles);
                self.bus.bus_trace.set_cycle(self.total_cycles);
                if self.logic_capture.push_active() {
                    self.bus.logic_tap.set_clock(self.total_cycles);
                }
                let executed =
                    self.cpu
                        .step_batch(&mut self.bus, &self.observers, &self.config, count)?;
                if executed == 0 {
                    self.bus.set_current_cycle(self.total_cycles);
                    self.bus.bus_trace.set_cycle(self.total_cycles);
                    if self.logic_capture.push_active() {
                        self.bus.logic_tap.set_clock(self.total_cycles + 1);
                    }
                }
                return Ok(CoreProgress {
                    primary_steps: executed,
                    secondary_steps: 0,
                });
            }
        }

        if self.cpu_secondary.is_none() {
            self.cpu
                .step(&mut self.bus, &self.observers, &self.config)?;
            return Ok(CoreProgress {
                primary_steps: 1,
                secondary_steps: 0,
            });
        }

        self.cpu
            .step(&mut self.bus, &self.observers, &self.config)?;
        self.release_secondary_cpu_if_requested();
        if let Err(error) =
            self.cpu_secondary
                .as_mut()
                .unwrap()
                .step(&mut self.bus, &self.observers, &self.config)
        {
            self.record_cpu_progress(1);
            return Err(error);
        }
        Ok(CoreProgress {
            primary_steps: 1,
            secondary_steps: 1,
        })
    }

    pub(crate) fn record_cpu_progress(&mut self, primary_steps: u32) {
        self.step_profile.cpu_instructions += u64::from(primary_steps);
        self.step_profile.cpu_batches += 1;
    }

    fn release_secondary_cpu_if_requested(&mut self) {
        let Some(cpu1) = self.cpu_secondary.as_mut() else {
            return;
        };
        if let Some(boot_addr) =
            crate::peripherals::esp_xtensa_common::rom_thunks::APPCPU_BOOT_ADDR.with(|s| s.take())
        {
            cpu1.set_pc(boot_addr);
            cpu1.unhalt();
        }
    }

    pub(crate) fn commit_advance_boundary(
        &mut self,
        mode: ExecutionMode,
        _batch_start: u64,
        progress: CoreProgress,
    ) -> SimResult<()> {
        if mode == ExecutionMode::RunBatch {
            self.total_cycles += u64::from(progress.primary_steps);
        }
        self.record_cpu_progress(progress.primary_steps);

        #[cfg(feature = "event-scheduler")]
        if mode == ExecutionMode::RunBatch {
            self.bus.set_current_cycle(_batch_start);
        }

        let logic_boundary = self.total_cycles;
        let tick_interval = u64::from(self.config.peripheral_tick_interval.max(1));
        if self.total_cycles % tick_interval == 0 {
            // Propagate peripherals
            let (interrupts, costs) = self.bus.tick_peripherals_fully();
            self.record_peripheral_tick_profile(costs.len());
            for c in costs {
                self.total_cycles += c.cycles as u64;
                if let Some(p) = self.bus.peripherals.get(c.index) {
                    for observer in &self.observers {
                        observer.on_peripheral_tick(&p.name, c.cycles);
                    }
                }
            }
            for irq in interrupts {
                self.cpu.set_exception_pending(irq);
                tracing::debug!("Exception {} Pend", irq);
            }
        }

        // Phase 2B.1 (issue #192): event-driven peripheral scheduler.
        // With the `event-scheduler` flag OFF this block compiles out
        // entirely and behaviour matches pre-2B `main`. With the flag ON
        // and no peripheral opted in (`uses_scheduler() == false` for
        // everyone) the drain is a no-op — the legacy `tick()` walk
        // above still drives every peripheral until each migrates.
        #[cfg(feature = "event-scheduler")]
        {
            self.bus.set_current_cycle(self.total_cycles);
            self.drain_scheduler_events();
        }

        // RTC_CNTL software system reset (OPTIONS0 bit 31 / `SW_SYS_RST`).
        // The ESP32 BROM's `_rtc_trigger_sw_system_reset` writes this bit
        // and expects execution NOT to return from the store — on real
        // silicon the CPU restarts at the reset vector. We drain the
        // request between instructions so neither the CPU nor any
        // peripheral observes a half-applied state. Reset vector for the
        // ESP32 rev3 BROM `_ResetVector` is fixed at `0x4000_0400`; SP is
        // re-seeded to the top of DRAM the BROM uses (`0x3FFE_0000`),
        // matching the smoke-test cold-boot setup.
        if self.drain_rtc_cntl_reset_request() {
            self.cpu.set_pc(0x4000_0400);
            self.cpu.set_sp(0x3FFE_0000);
            tracing::debug!("RTC_CNTL SW_SYS_RST: CPU re-pointed at reset vector 0x40000400");
        }

        // Cortex-M SCB system reset (AIRCR.SYSRESETREQ with the VECTKEY).
        // Firmware that asks for a reboot (e.g. a UDS ECUReset) writes
        // AIRCR and does not expect the store to return; on real silicon the
        // core restarts through the vector table. We drain the latch here, at
        // the same clean instruction boundary as RTC_CNTL — after the
        // AIRCR-writing store and any pending peripheral effects of this
        // instruction have been applied — then reuse the power-on reset
        // machinery so MSP/PC reload from vector[0]/vector[1] via the CPU
        // reset path. No-op on non-Cortex-M targets (no SCB on the bus).
        if self.drain_scb_reset_request() {
            // AIRCR.SYSRESETREQ is self-clearing across the reset. Clear the
            // modeled readback bit too so later boundaries do not retrigger it.
            if let Some(index) = self.scb_index {
                self.bus.peripherals[index].dev.write_u32(0x0c, 0)?;
            }
            self.reset()?;
            tracing::debug!("SCB SYSRESETREQ: CPU rebooted through vector table");
        }

        // H5 FLASH pending ops: sector erase fills flash with 0xFF; bank-swap
        // swaps the two 1 MB banks in the flash buffer then re-runs reset so
        // the CPU boots from the new bank-1 vector table. Also drained on the
        // batch/CLI run path (`Machine::run`), which executes cycle-accurately
        // when an H5 op-modeling FLASH is present so this fires per instruction.
        self.apply_pending_flash_op()?;

        // Logic-analyzer edge capture. No-op (one `is_active` check) unless a
        // watch set is installed. Observed after the instruction + peripheral
        // effects of this cycle have landed, so pad levels are the committed
        // state at `total_cycles`.
        self.logic_observe(logic_boundary);

        Ok(())
    }
}
