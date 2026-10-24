// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

#[cfg(test)]
mod peripheral_reset_request_tests {
    use crate::{Bus, Cpu, Machine, Peripheral, SimResult};

    /// A minimal peripheral that latches a reset request when offset 0 is
    /// written non-zero, the way a watchdog latches on a time-out.
    #[derive(Debug, Default)]
    struct ResetOnWrite {
        latch: bool,
    }

    impl Peripheral for ResetOnWrite {
        fn read(&self, _offset: u64) -> SimResult<u8> {
            Ok(0)
        }

        fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
            if offset == 0 && value != 0 {
                self.latch = true;
            }
            Ok(())
        }

        fn take_reset_request(&mut self) -> bool {
            std::mem::take(&mut self.latch)
        }

        fn latched_reset_cause(&self) -> Option<crate::PeripheralResetCause> {
            self.latch.then_some(crate::PeripheralResetCause::Watchdog)
        }

        fn reset_request_capable(&self) -> bool {
            true
        }
    }

    /// Records the cause the machine publishes after it drains a request.
    /// The test holds the same `Arc` the peripheral writes, so the assertion
    /// does not downcast the bus device. `Peripheral` requires `Debug + Send`.
    #[derive(Debug)]
    struct CauseListener {
        seen: std::sync::Arc<std::sync::Mutex<Option<crate::PeripheralResetCause>>>,
    }

    impl Peripheral for CauseListener {
        fn read(&self, _offset: u64) -> SimResult<u8> {
            Ok(0)
        }

        fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
            Ok(())
        }

        fn on_peripheral_reset(&mut self, cause: crate::PeripheralResetCause) {
            *self.seen.lock().expect("cause lock") = Some(cause);
        }
    }

    const PERIPH_BASE: u64 = 0x4000_0000;

    /// A peripheral that requests a CPU reset must reboot the CPU through the
    /// vector table at the next instruction boundary, using the same path as
    /// SYSRESETREQ: MSP from vector[0], PC from vector[1].
    #[test]
    fn peripheral_reset_request_reboots_cpu_via_vector_table() {
        let mut bus = crate::bus::SystemBus::new();
        let (cpu, _nvic) = crate::system::cortex_m::configure_cortex_m(&mut bus);
        // Mount before `Machine::new`: the reset-capable index cache is built
        // at construction.
        bus.add_peripheral(
            "reset_on_write",
            PERIPH_BASE,
            0x100,
            None,
            Box::new(ResetOnWrite::default()),
        );
        let seen = std::sync::Arc::new(std::sync::Mutex::new(None));
        bus.add_peripheral(
            "cause_listener",
            PERIPH_BASE + 0x100,
            0x100,
            None,
            Box::new(CauseListener {
                seen: std::sync::Arc::clone(&seen),
            }),
        );
        let mut m = Machine::new(cpu, bus);

        const MSP: u32 = 0x2000_1000;
        const RESET_ADDR: u32 = 0x0800_0100;
        m.bus.write_u32(0x0000_0000, MSP).unwrap();
        m.bus.write_u32(0x0000_0004, RESET_ADDR | 1).unwrap();

        // One NOP at PC so the step completes an instruction before the
        // latch is drained, mirroring the SCB test.
        const PC: u32 = 0x2000_0000;
        m.bus.write_u16(PC as u64, 0xBF00).unwrap();
        m.cpu.set_pc(PC);
        m.cpu.set_sp(0x2000_8000);

        // Latch the request through a real bus write, as firmware would.
        m.bus.write_u8(PERIPH_BASE, 1).unwrap();

        m.step().unwrap();

        assert_eq!(
            m.cpu.get_pc() & !1,
            RESET_ADDR,
            "PC must reload from vector[1] after a peripheral reset request"
        );
        assert_eq!(
            m.cpu.get_register(13),
            MSP,
            "SP must reload from vector[0] after a peripheral reset request"
        );
        assert_eq!(
            *seen.lock().expect("cause lock"),
            Some(crate::PeripheralResetCause::Watchdog)
        );
    }
}
