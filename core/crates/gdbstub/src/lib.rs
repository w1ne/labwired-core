// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use core::convert::Infallible;
use gdbstub::stub::{BaseStopReason, GdbStub};
use gdbstub::target::ext::base::singlethread::{
    SingleThreadBase, SingleThreadResume, SingleThreadSingleStep,
};
use gdbstub::target::ext::base::BaseOps;
use gdbstub::target::{Target, TargetError, TargetResult};
use labwired_core::cpu::{CortexM, RiscV};
use labwired_core::{Cpu, DebugControl, Machine, StopReason};
use std::marker::PhantomData;
use std::net::{TcpListener, TcpStream};

pub struct LabwiredTarget<C: Cpu> {
    pub machine: Machine<C>,
}

impl<C: Cpu> LabwiredTarget<C> {
    pub fn new(machine: Machine<C>) -> Self {
        Self { machine }
    }
}

impl Target for LabwiredTarget<CortexM> {
    type Arch = gdbstub_arch::arm::Armv4t;
    type Error = Infallible;

    fn base_ops(&mut self) -> BaseOps<'_, Self::Arch, Self::Error> {
        BaseOps::SingleThread(self)
    }

    fn support_breakpoints(
        &mut self,
    ) -> Option<gdbstub::target::ext::breakpoints::BreakpointsOps<'_, Self>> {
        Some(self)
    }
}

impl SingleThreadBase for LabwiredTarget<CortexM> {
    fn read_registers(
        &mut self,
        regs: &mut gdbstub_arch::arm::reg::ArmCoreRegs,
    ) -> TargetResult<(), Self> {
        for i in 0..13 {
            regs.r[i] = self.machine.read_core_reg(i as u8);
        }
        regs.sp = self.machine.read_core_reg(13);
        regs.lr = self.machine.read_core_reg(14);
        regs.pc = self.machine.read_core_reg(15);
        regs.cpsr = self.machine.read_core_reg(16); // xPSR
        Ok(())
    }

    fn write_registers(
        &mut self,
        regs: &gdbstub_arch::arm::reg::ArmCoreRegs,
    ) -> TargetResult<(), Self> {
        for i in 0..13 {
            self.machine.write_core_reg(i as u8, regs.r[i]);
        }
        self.machine.write_core_reg(13, regs.sp);
        self.machine.write_core_reg(14, regs.lr);
        self.machine.write_core_reg(15, regs.pc);
        self.machine.write_core_reg(16, regs.cpsr);
        Ok(())
    }

    fn read_addrs(&mut self, start_addr: u32, data: &mut [u8]) -> TargetResult<usize, Self> {
        let mem = self
            .machine
            .read_memory(start_addr, data.len())
            .map_err(|_| TargetError::NonFatal)?;
        let len = mem.len().min(data.len());
        data[..len].copy_from_slice(&mem[..len]);
        Ok(len)
    }

    fn write_addrs(&mut self, start_addr: u32, data: &[u8]) -> TargetResult<(), Self> {
        self.machine
            .write_memory(start_addr, data)
            .map_err(|_| TargetError::NonFatal)?;
        Ok(())
    }

    fn support_resume(
        &mut self,
    ) -> Option<gdbstub::target::ext::base::singlethread::SingleThreadResumeOps<'_, Self>> {
        Some(self)
    }
}

impl Target for LabwiredTarget<RiscV> {
    type Arch = gdbstub_arch::riscv::Riscv32;
    type Error = Infallible;

    fn base_ops(&mut self) -> BaseOps<'_, Self::Arch, Self::Error> {
        BaseOps::SingleThread(self)
    }

    fn support_breakpoints(
        &mut self,
    ) -> Option<gdbstub::target::ext::breakpoints::BreakpointsOps<'_, Self>> {
        Some(self)
    }
}

impl SingleThreadBase for LabwiredTarget<RiscV> {
    fn read_registers(
        &mut self,
        regs: &mut gdbstub_arch::riscv::reg::RiscvCoreRegs<u32>,
    ) -> TargetResult<(), Self> {
        for i in 0..32 {
            regs.x[i] = self.machine.read_core_reg(i as u8);
        }
        regs.pc = self.machine.read_core_reg(32); // Assuming read_core_reg(32) is PC for RISC-V
        Ok(())
    }

    fn write_registers(
        &mut self,
        regs: &gdbstub_arch::riscv::reg::RiscvCoreRegs<u32>,
    ) -> TargetResult<(), Self> {
        for i in 0..32 {
            self.machine.write_core_reg(i as u8, regs.x[i]);
        }
        self.machine.write_core_reg(32, regs.pc);
        Ok(())
    }

    fn read_addrs(
        &mut self,
        start_addr: <Self::Arch as gdbstub::arch::Arch>::Usize,
        data: &mut [u8],
    ) -> TargetResult<usize, Self> {
        let mem = self
            .machine
            .read_memory(start_addr, data.len())
            .map_err(|_| TargetError::NonFatal)?;
        let len = mem.len().min(data.len());
        data[..len].copy_from_slice(&mem[..len]);
        Ok(len)
    }

    fn write_addrs(
        &mut self,
        start_addr: <Self::Arch as gdbstub::arch::Arch>::Usize,
        data: &[u8],
    ) -> TargetResult<(), Self> {
        self.machine
            .write_memory(start_addr, data)
            .map_err(|_| TargetError::NonFatal)?;
        Ok(())
    }

    fn support_resume(
        &mut self,
    ) -> Option<gdbstub::target::ext::base::singlethread::SingleThreadResumeOps<'_, Self>> {
        Some(self)
    }
}

impl<C: Cpu> SingleThreadResume for LabwiredTarget<C>
where
    LabwiredTarget<C>: Target<Arch: gdbstub::arch::Arch<Usize = u32>>,
{
    fn resume(&mut self, _signal: Option<gdbstub::common::Signal>) -> Result<(), Self::Error> {
        Ok(())
    }

    fn support_single_step(
        &mut self,
    ) -> Option<gdbstub::target::ext::base::singlethread::SingleThreadSingleStepOps<'_, Self>> {
        Some(self)
    }
}

impl<C: Cpu> SingleThreadSingleStep for LabwiredTarget<C>
where
    LabwiredTarget<C>: Target<Arch: gdbstub::arch::Arch<Usize = u32>>,
{
    fn step(&mut self, _signal: Option<gdbstub::common::Signal>) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl<C: Cpu> gdbstub::target::ext::breakpoints::Breakpoints for LabwiredTarget<C>
where
    LabwiredTarget<C>: Target<Arch: gdbstub::arch::Arch<Usize = u32>>,
{
    fn support_sw_breakpoint(
        &mut self,
    ) -> Option<gdbstub::target::ext::breakpoints::SwBreakpointOps<'_, Self>> {
        Some(self)
    }
}

impl<C: Cpu> gdbstub::target::ext::breakpoints::SwBreakpoint for LabwiredTarget<C>
where
    LabwiredTarget<C>: Target<Arch: gdbstub::arch::Arch<Usize = u32>>,
{
    fn add_sw_breakpoint(
        &mut self,
        addr: u32,
        _kind: <Self::Arch as gdbstub::arch::Arch>::BreakpointKind,
    ) -> TargetResult<bool, Self> {
        self.machine.add_breakpoint(addr);
        Ok(true)
    }

    fn remove_sw_breakpoint(
        &mut self,
        addr: u32,
        _kind: <Self::Arch as gdbstub::arch::Arch>::BreakpointKind,
    ) -> TargetResult<bool, Self> {
        self.machine.remove_breakpoint(addr);
        Ok(true)
    }
}

pub struct GdbServer {
    port: u16,
}

impl GdbServer {
    pub fn new(port: u16) -> Self {
        Self { port }
    }

    pub fn run<C: Cpu + 'static>(&self, machine: Machine<C>) -> anyhow::Result<()>
    where
        LabwiredTarget<C>: Target<Error = Infallible, Arch: gdbstub::arch::Arch<Usize = u32>>,
        GdbEventLoop<C>: gdbstub::stub::run_blocking::BlockingEventLoop<
            Target = LabwiredTarget<C>,
            Connection = TcpStream,
            StopReason = BaseStopReason<(), u32>,
        >,
    {
        let listener = TcpListener::bind(format!("0.0.0.0:{}", self.port))?;
        tracing::info!("GDB server listening on 0.0.0.0:{}", self.port);

        let (stream, addr) = listener.accept()?;
        tracing::info!("GDB client connected from {}", addr);

        let mut target = LabwiredTarget::new(machine);
        let gdb = GdbStub::new(stream);

        match gdb.run_blocking::<GdbEventLoop<C>>(&mut target) {
            Ok(reason) => tracing::info!("GDB session ended: {:?}", reason),
            Err(e) => tracing::error!("GDB session error: {:?}", e),
        }

        Ok(())
    }
}

pub struct GdbEventLoop<C: Cpu>(PhantomData<C>);

impl<C: Cpu> gdbstub::stub::run_blocking::BlockingEventLoop for GdbEventLoop<C>
where
    LabwiredTarget<C>: Target<Arch: gdbstub::arch::Arch<Usize = u32>>,
{
    type Target = LabwiredTarget<C>;
    type Connection = TcpStream;
    type StopReason = BaseStopReason<(), u32>;

    fn wait_for_stop_reason(
        target: &mut Self::Target,
        conn: &mut Self::Connection,
    ) -> Result<
        gdbstub::stub::run_blocking::Event<Self::StopReason>,
        gdbstub::stub::run_blocking::WaitForStopReasonError<
            <Self::Target as Target>::Error,
            <Self::Connection as gdbstub::conn::Connection>::Error,
        >,
    > {
        use gdbstub::stub::run_blocking::Event;
        use std::io::Read;

        loop {
            // Non-blocking peep at connection for interrupt
            let mut byte = [0];
            conn.set_nonblocking(true).ok();
            let incoming = match conn.read(&mut byte) {
                Ok(1) => {
                    conn.set_nonblocking(false).ok();
                    Some(byte[0])
                }
                _ => {
                    conn.set_nonblocking(false).ok();
                    None
                }
            };

            if let Some(b) = incoming {
                return Ok(Event::IncomingData(b));
            }

            // Run machine for a small chunk
            match target.machine.run(Some(1000)) {
                Ok(StopReason::Breakpoint(_)) => {
                    return Ok(Event::TargetStopped(BaseStopReason::Signal(
                        gdbstub::common::Signal::SIGTRAP,
                    )))
                }
                Ok(StopReason::StepDone) => {
                    return Ok(Event::TargetStopped(BaseStopReason::Signal(
                        gdbstub::common::Signal::SIGTRAP,
                    )))
                }
                Ok(_) => {
                    // MaxSteps reached, continue loop and check for interrupt again
                    continue;
                }
                Err(e) => {
                    tracing::error!("GDB Simulation Error: {}", e);
                    return Ok(Event::TargetStopped(BaseStopReason::Signal(
                        gdbstub::common::Signal::SIGSEGV,
                    )));
                }
            }
        }
    }

    fn on_interrupt(
        _target: &mut Self::Target,
    ) -> Result<Option<Self::StopReason>, <Self::Target as Target>::Error> {
        Ok(Some(BaseStopReason::Signal(
            gdbstub::common::Signal::SIGINT,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use labwired_core::bus::SystemBus;
    use labwired_core::cpu::CortexM;

    #[test]
    fn test_target_register_access() {
        let mut bus = SystemBus::new();
        let (cpu, _nvic) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
        let machine = Machine::new(cpu, bus);
        let mut target = LabwiredTarget::<CortexM>::new(machine);

        // Mock some register values
        target.machine.write_core_reg(0, 0x12345678);
        target.machine.write_core_reg(15, 0x08000100);
        target.machine.write_core_reg(16, 0x60000000); // xPSR

        let mut regs = gdbstub_arch::arm::reg::ArmCoreRegs::default();
        target
            .read_registers(&mut regs)
            .unwrap_or_else(|_| panic!("Failed to read registers"));

        assert_eq!(regs.r[0], 0x12345678);
        assert_eq!(regs.pc, 0x08000100);
        assert_eq!(regs.cpsr, 0x60000000);

        // Test write
        regs.r[1] = 0xdeadbeef;
        target
            .write_registers(&regs)
            .unwrap_or_else(|_| panic!("Failed to write registers"));
        assert_eq!(target.machine.read_core_reg(1), 0xdeadbeef);
    }

    #[test]
    fn test_riscv_target_register_access() {
        let bus = SystemBus::new();
        let cpu = labwired_core::cpu::RiscV::new();
        let machine = Machine::new(cpu, bus);
        let mut target = LabwiredTarget::<RiscV>::new(machine);

        // Mock some register values
        target.machine.write_core_reg(1, 0x12345678); // x1
        target.machine.write_core_reg(32, 0x80000100); // PC

        let mut regs = gdbstub_arch::riscv::reg::RiscvCoreRegs::<u32>::default();
        target
            .read_registers(&mut regs)
            .unwrap_or_else(|_| panic!("Failed to read registers"));

        assert_eq!(regs.x[1], 0x12345678);
        assert_eq!(regs.pc, 0x80000100);

        // Test write
        regs.x[2] = 0xdeadbeef;
        target
            .write_registers(&regs)
            .unwrap_or_else(|_| panic!("Failed to write registers"));
        assert_eq!(target.machine.read_core_reg(2), 0xdeadbeef);
    }

    #[test]
    fn test_target_memory_access() {
        let mut bus = SystemBus::new();
        let (cpu, _nvic) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
        let machine = Machine::new(cpu, bus);
        let mut target = LabwiredTarget::<CortexM>::new(machine);

        let data = [0xAA, 0xBB, 0xCC, 0xDD];
        // Memory write/read test would ideally go to a peripheral but we just want to verify trait calls.
        // Direct memory access via target.write_addrs should not panic if it fails gracefully.
        let _ = target.write_addrs(0x20000000, &data);
    }
}
