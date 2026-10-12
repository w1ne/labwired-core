#![cfg(feature = "qemu-arm-system")]
use labwired_core::cpu::CortexA7System;
use labwired_core::{
    Bus, Cpu, DmaRequest, SimResult, SimulationConfig, SimulationError, SimulationObserver,
};
use labwired_hw_trace::TraceEvent;
use std::cell::RefCell;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
struct TestBus {
    id: Option<u64>,
    ram: Vec<u8>,
    config: SimulationConfig,
    accesses: RefCell<Vec<(bool, u64, u32)>>,
    io: u64,
    levels: Option<(bool, bool)>,
    cp: RefCell<Vec<(u32, bool, u64)>>,
    cp_value: Option<u64>,
    fail: bool,
    now: u64,
    deadline: Option<u64>,
}
impl TestBus {
    fn new(code: &[u32]) -> Self {
        let mut b = Self {
            id: Some(NEXT_ID.fetch_add(1, Ordering::Relaxed)),
            ram: vec![0; 0x10000],
            config: SimulationConfig::default(),
            accesses: RefCell::new(vec![]),
            io: 0,
            levels: Some((false, false)),
            cp: RefCell::new(vec![]),
            cp_value: None,
            fail: false,
            now: 0,
            deadline: None,
        };
        b.words(0x100, code);
        b
    }
    fn words(&mut self, a: usize, ws: &[u32]) {
        for (i, w) in ws.iter().enumerate() {
            self.ram[a + i * 4..a + i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
    }
    fn read(&self, a: u64, w: u32) -> SimResult<u64> {
        if self.fail {
            return Err(SimulationError::Other("injected read failure".into()));
        }
        self.accesses.borrow_mut().push((false, a, w));
        if a == 0x100000 {
            return Ok(self.io);
        }
        let p = usize::try_from(a).map_err(|_| SimulationError::MemoryViolation(a))?;
        let bytes = self
            .ram
            .get(
                p..p.checked_add(w as usize)
                    .ok_or(SimulationError::MemoryViolation(a))?,
            )
            .ok_or(SimulationError::MemoryViolation(a))?;
        let mut v = [0; 8];
        v[..bytes.len()].copy_from_slice(bytes);
        Ok(u64::from_le_bytes(v))
    }
    fn write(&mut self, a: u64, w: u32, v: u64) -> SimResult<()> {
        self.accesses.borrow_mut().push((true, a, w));
        if a == 0x100000 {
            self.io = v;
            return Ok(());
        }
        let p = a as usize;
        self.ram
            .get_mut(p..p + w as usize)
            .ok_or(SimulationError::MemoryViolation(a))?
            .copy_from_slice(&v.to_le_bytes()[..w as usize]);
        Ok(())
    }
}
impl Bus for TestBus {
    fn read_u8(&self, a: u64) -> SimResult<u8> {
        Ok(self.read(a, 1)? as u8)
    }
    fn read_u16(&self, a: u64) -> SimResult<u16> {
        Ok(self.read(a, 2)? as u16)
    }
    fn read_u32(&self, a: u64) -> SimResult<u32> {
        Ok(self.read(a, 4)? as u32)
    }
    fn read_u64(&self, a: u64) -> SimResult<u64> {
        self.read(a, 8)
    }
    fn write_u8(&mut self, a: u64, v: u8) -> SimResult<()> {
        self.write(a, 1, v as u64)
    }
    fn write_u16(&mut self, a: u64, v: u16) -> SimResult<()> {
        self.write(a, 2, v as u64)
    }
    fn write_u32(&mut self, a: u64, v: u32) -> SimResult<()> {
        self.write(a, 4, v as u64)
    }
    fn write_u64(&mut self, a: u64, v: u64) -> SimResult<()> {
        self.write(a, 8, v)
    }
    fn arm_code_identity(&self) -> Option<u64> {
        self.id
    }
    fn arm_irq_fiq_levels(&self) -> Option<(bool, bool)> {
        self.levels
    }
    fn arm_cp15_read(&self, k: u32, w: bool) -> SimResult<u64> {
        self.cp.borrow_mut().push((k, w, 0));
        self.cp_value
            .ok_or(SimulationError::NotImplemented("external CP15".into()))
    }
    fn arm_cp15_write(&mut self, k: u32, w: bool, v: u64) -> SimResult<()> {
        self.cp.borrow_mut().push((k, w, v));
        self.cp_value
            .map(|_| ())
            .ok_or(SimulationError::NotImplemented("external CP15".into()))
    }
    fn current_cycle(&self) -> u64 {
        self.now
    }
    fn publish_cycle(&mut self, c: u64) {
        self.now = c;
    }
    fn next_cpu_interrupt_deadline_cycle(&self) -> Option<u64> {
        self.deadline
    }
    fn tick_peripherals(&mut self) -> Vec<u32> {
        vec![]
    }
    fn execute_dma(&mut self, _: &[DmaRequest]) -> SimResult<()> {
        Ok(())
    }
    fn config(&self) -> &SimulationConfig {
        &self.config
    }
}
fn cpu() -> CortexA7System {
    let mut c = CortexA7System::new().unwrap();
    c.executable_range(0, 0x10000).unwrap();
    c.set_pc(0x100);
    c
}
fn run(c: &mut dyn Cpu, b: &mut TestBus, n: u32) -> u32 {
    c.step_batch(b, &[], &SimulationConfig::default(), n)
        .unwrap()
}
#[derive(Debug, Default)]
struct Trace(Mutex<Vec<(u32, u32)>>, Mutex<Vec<&'static str>>);
impl SimulationObserver for Trace {
    fn on_step_start(&self, p: u32, o: u32) {
        self.1.lock().unwrap().push("start");
        self.0.lock().unwrap().push((p, o));
    }
    fn on_trace_event(&self, e: TraceEvent) {
        if let TraceEvent::InstructionRetired { pc, opcode } = e {
            self.1.lock().unwrap().push("retired");
            assert_eq!(self.0.lock().unwrap().last(), Some(&(pc, opcode)));
        }
    }
    fn on_step_end(&self, _: u32, _: &[u32]) {
        self.1.lock().unwrap().push("end");
    }
}
#[test]
fn arithmetic_mmio_preserves_width() {
    let mut b = TestBus::new(&[0xe3a00029, 0xe2800001, 0xe5810000, 0xe5d12000]);
    let mut c = cpu();
    c.set_register(1, 0x100000);
    assert_eq!(run(&mut c, &mut b, 4), 4);
    assert_eq!(c.get_register(0), 42);
    assert_eq!(b.io, 42);
    assert!(b.accesses.borrow().contains(&(true, 0x100000, 4)));
    assert!(b.accesses.borrow().contains(&(false, 0x100000, 1)));
}
#[test]
fn mmu_uses_physical_bus() {
    let mut b = TestBus::new(&[
        0xee020f10, 0xee031f10, 0xee112f10, 0xe3822001, 0xee012f10, 0xe5934000,
    ]);
    b.words(0x300, &[42]);
    b.words(0x4000, &[0xc02]);
    b.words(0x6000, &[0xc02]);
    let mut c = cpu();
    c.set_register(0, 0x4000);
    c.set_register(1, 3);
    c.set_register(3, 0x80000300);
    assert_eq!(run(&mut c, &mut b, 6), 6);
    assert_eq!(c.get_register(4), 42);
    assert!(b.accesses.borrow().iter().any(|x| x.1 == 0x300));
    assert!(!b.accesses.borrow().iter().any(|x| x.1 == 0x80000300));
}
#[test]
fn svc_and_irq_emit_real_handler_instruction() {
    let mut b = TestBus::new(&[0xef000000]);
    b.words(8, &[0xe3a05007]);
    b.words(0x18, &[0xe3a0600d, 0xe3a0702a]);
    let mut c = cpu();
    assert_eq!(run(&mut c, &mut b, 2), 2);
    assert_eq!(c.get_register(5), 7);
    c.set_register(16, 0x13);
    b.levels = Some((true, false));
    let t = Arc::new(Trace::default());
    assert_eq!(
        c.step_retired(&mut b, &[t.clone()], &SimulationConfig::default())
            .unwrap(),
        1
    );
    assert_eq!(*t.0.lock().unwrap(), vec![(0x18, 0xe3a0600d)]);
    assert_eq!(*t.1.lock().unwrap(), vec!["start", "retired", "end"]);
    assert_eq!(c.get_register(16) & 31, 0x12);
    b.levels = Some((false, false));
    assert_eq!(run(&mut c, &mut b, 1), 1);
    assert_eq!(c.get_register(7), 42);
}
#[test]
fn wfi_wait_has_zero_retirement_and_real_deadline() {
    let mut b = TestBus::new(&[0xe320f003, 0xe3a0002a]);
    let mut c = cpu();
    assert_eq!(run(&mut c, &mut b, 1), 1);
    assert!(c.is_parked_idle());
    let t = Arc::new(Trace::default());
    assert_eq!(
        c.step_retired(&mut b, &[t.clone()], &SimulationConfig::default())
            .unwrap(),
        0
    );
    assert!(t.0.lock().unwrap().is_empty());
    assert_eq!(c.idle_fast_forward_budget(&b), None);
    b.now = 7;
    b.deadline = Some(19);
    assert_eq!(c.idle_fast_forward_budget(&b), Some(12));
    b.levels = Some((true, false));
    assert_eq!(c.idle_fast_forward_budget(&b), None);
}
#[test]
fn callback_failures_and_pending_setter_errors_surface() {
    let mut b = TestBus::new(&[0xe3a0002a]);
    let mut c = cpu();
    b.fail = true;
    assert!(c
        .step_retired(&mut b, &[], &SimulationConfig::default())
        .unwrap_err()
        .to_string()
        .contains("injected read failure"));
    b.fail = false;
    c.reset(&mut b).unwrap();
    c.set_pc(0x100);
    c.set_register(99, 1);
    assert!(c
        .step_retired(&mut b, &[], &SimulationConfig::default())
        .is_err());
    c.reset(&mut b).unwrap();
    c.set_pc(0x100);
    c.set_exception_pending(16);
    assert!(c
        .step_retired(&mut b, &[], &SimulationConfig::default())
        .is_err());
}
#[test]
fn unsupported_bus_identity_and_snapshots_refuse() {
    let mut b = TestBus::new(&[0xe3a0002a]);
    let mut c = cpu();
    assert!(c.try_apply_snapshot(&c.snapshot()).is_err());
    assert!(c.runtime_snapshot().is_none());
    b.id = None;
    assert!(matches!(
        c.step_retired(&mut b, &[], &SimulationConfig::default()),
        Err(SimulationError::NotImplemented(_))
    ));
}
#[test]
fn bus_swap_and_host_invalidation_use_current_code() {
    let mut b = TestBus::new(&[0xe3a00001]);
    let mut c = cpu();
    assert_eq!(run(&mut c, &mut b, 1), 1);
    let mut other = TestBus::new(&[0xe3a00002]);
    c.set_pc(0x100);
    run(&mut c, &mut other, 1);
    assert_eq!(c.get_register(0), 2);
    other.words(0x100, &[0xe3a00003]);
    c.invalidate_code_caches();
    c.set_pc(0x100);
    run(&mut c, &mut other, 1);
    assert_eq!(c.get_register(0), 3);
}
#[test]
fn cp15_callback_writes_end_window() {
    let mut b = TestBus::new(&[0xee1e0f10, 0xee0e0f10, 0xec510f0e, 0xec410f0e]);
    b.cp_value = Some(0x1122334455667788);
    let mut c = cpu();
    assert_eq!(run(&mut c, &mut b, 4), 2);
    assert_eq!(b.cp.borrow()[1], (0xe0000, false, 0x55667788));
    assert_eq!(run(&mut c, &mut b, 2), 2);
    assert_eq!(b.cp.borrow()[3], (0xe, true, 0x1122334455667788));
    let mut b = TestBus::new(&[0xee1e0f10]);
    let mut c = cpu();
    assert!(matches!(
        c.step_retired(&mut b, &[], &SimulationConfig::default()),
        Err(SimulationError::NotImplemented(_))
    ));
}
#[test]
fn vfp_keeps_eight_byte_transaction() {
    let mut b = TestBus::new(&[
        0xee110f50, 0xe380060f, 0xee010f50, 0xe3a00101, 0xeee80a10, 0xed910b00, 0xed820b00,
    ]);
    b.words(0x300, &[0x55667788, 0x11223344]);
    let mut c = cpu();
    c.set_register(1, 0x300);
    c.set_register(2, 0x308);
    for _ in 0..7 {
        run(&mut c, &mut b, 1);
    }
    assert_eq!(&b.ram[0x308..0x310], &0x1122334455667788u64.to_le_bytes());
    assert!(b.accesses.borrow().contains(&(false, 0x300, 8)));
    assert!(b.accesses.borrow().contains(&(true, 0x308, 8)));
}
#[test]
fn observer_opcode_is_translated_at_virtual_pc() {
    let mut b = TestBus::new(&[0xee020f10, 0xee031f10, 0xee112f10, 0xe3822001, 0xee012f10]);
    b.words(0x300, &[0xe3a0402a]);
    b.words(0x4000, &[0xc02]);
    b.words(0x6000, &[0xc02]);
    let mut c = cpu();
    c.set_register(0, 0x4000);
    c.set_register(1, 3);
    run(&mut c, &mut b, 5);
    c.set_pc(0x80000300);
    let t = Arc::new(Trace::default());
    assert_eq!(
        c.step_retired(&mut b, &[t.clone()], &SimulationConfig::default())
            .unwrap(),
        1
    );
    assert_eq!(*t.0.lock().unwrap(), vec![(0x80000300, 0xe3a0402a)]);
    assert_eq!(c.get_register(4), 42);
    assert!(!b.accesses.borrow().iter().any(|x| x.1 == 0x80000300));
}
#[test]
fn system_bus_u64_is_ram_only_and_checks_whole_range() {
    use labwired_core::bus::SystemBus;
    let mut b = SystemBus::new();
    let a = b.ram.base_addr;
    assert_eq!(b.access_counts(), (0, 0, 0));
    b.write_u64(a, 0x1122334455667788).unwrap();
    assert_eq!(b.access_counts(), (0, 1, 0));
    assert_eq!(b.read_u64(a).unwrap(), 0x1122334455667788);
    assert_eq!(b.access_counts(), (1, 1, 0));
    let end = a + b.ram.data.len() as u64;
    let before = b.ram.data.clone();
    assert!(b.write_u64(end - 4, 0).is_err());
    assert_eq!(b.ram.data, before);
    assert!(matches!(
        b.read_u64(0x40000000),
        Err(SimulationError::NotImplemented(_))
    ));
    assert!(b.read_u64(u64::MAX).is_err());
    assert_eq!(b.access_counts(), (1, 1, 0));
}

#[test]
fn masked_irq_wakes_wfi_without_entering_irq_mode() {
    let mut b = TestBus::new(&[0xe320f003, 0xe3a0002a]);
    let mut c = cpu();
    c.set_register(16, 0xd3);
    assert_eq!(run(&mut c, &mut b, 1), 1);
    b.levels = Some((true, false));
    assert_eq!(run(&mut c, &mut b, 1), 1);
    assert_eq!(c.get_register(0), 42);
    assert_eq!(c.get_register(16) & 0x9f, 0x93);
    assert!(!c.is_parked_idle());
}

#[test]
fn fiq_entry_retires_only_vector_instruction() {
    let mut b = TestBus::new(&[0xe3a00001]);
    b.words(0x1c, &[0xe3a0602a]);
    let mut c = cpu();
    c.set_register(16, 0x13);
    b.levels = Some((false, true));
    let t = Arc::new(Trace::default());
    assert_eq!(
        c.step_retired(&mut b, &[t.clone()], &SimulationConfig::default())
            .unwrap(),
        1
    );
    assert_eq!(c.get_register(16) & 31, 0x11);
    assert_eq!(*t.0.lock().unwrap(), vec![(0x1c, 0xe3a0602a)]);
}

#[test]
fn boxed_cpu_forwards_zero_retirement_and_read_errors() {
    let mut b = TestBus::new(&[0xe320f003]);
    let mut c: Box<dyn Cpu> = Box::new(cpu());
    assert_eq!(run(&mut c, &mut b, 1), 1);
    assert_eq!(
        c.step_retired(&mut b, &[], &SimulationConfig::default())
            .unwrap(),
        0
    );
    assert_eq!(c.get_register(99), 0);
    assert!(c
        .step_retired(&mut b, &[], &SimulationConfig::default())
        .is_err());
}

#[derive(Debug)]
struct SideEffectRegister(Arc<AtomicU64>);
impl labwired_core::Peripheral for SideEffectRegister {
    fn read(&self, _: u64) -> SimResult<u8> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(0)
    }
    fn write(&mut self, _: u64, _: u8) -> SimResult<()> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}
#[test]
fn system_bus_refuses_mmio64_before_any_register_side_effect() {
    let mut b = labwired_core::bus::SystemBus::new();
    let calls = Arc::new(AtomicU64::new(0));
    b.add_peripheral(
        "side_effect",
        0x40000000,
        16,
        None,
        Box::new(SideEffectRegister(calls.clone())),
    );
    assert!(matches!(
        b.read_u64(0x40000000),
        Err(SimulationError::NotImplemented(_))
    ));
    assert!(matches!(
        b.write_u64(0x40000000, 1),
        Err(SimulationError::NotImplemented(_))
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[test]
fn self_branch_batch_retires_real_instructions_in_arm_and_thumb() {
    for thumb in [false, true] {
        let mut b = TestBus::new(&[if thumb { 0xe7fee7fe } else { 0xeafffffe }]);
        let mut c = cpu();
        if thumb {
            c.set_register(16, 0x33);
        }
        assert_eq!(run(&mut c, &mut b, 8), 8);
        assert_eq!(c.get_pc(), 0x100);
        assert!(!c.is_parked_idle());
        let t = Arc::new(Trace::default());
        assert_eq!(
            c.step_retired(&mut b, &[t.clone()], &SimulationConfig::default())
                .unwrap(),
            1
        );
        assert_eq!(
            *t.0.lock().unwrap(),
            vec![(0x100, if thumb { 0xe7fe } else { 0xeafffffe })]
        );
    }
}

#[test]
fn unmapped_abort_vector_returns_poisoning_error_instead_of_hanging() {
    let mut b = TestBus::new(&[0xee020f10, 0xee031f10, 0xee112f10, 0xe3822001, 0xee012f10]);
    let mut c = cpu();
    c.set_register(0, 0x4000);
    c.set_register(1, 3);
    run(&mut c, &mut b, 5);
    let error = c
        .step_retired(&mut b, &[], &SimulationConfig::default())
        .unwrap_err();
    assert!(error.to_string().contains("without retirement"));
    assert!(c
        .step_retired(&mut b, &[], &SimulationConfig::default())
        .is_err());
    c.reset(&mut b).unwrap();
    c.set_pc(0x100);
    assert_eq!(run(&mut c, &mut b, 1), 1);
}
