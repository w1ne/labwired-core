use crate::{Bus, Cpu, DebugControl, Machine, SimResult, SimulationObserver, StopReason};
use std::sync::Arc;

#[allow(dead_code)]
#[derive(Default)]
struct MockCpu {
    pc: u32,
}

impl Cpu for MockCpu {
    fn reset(&mut self, _bus: &mut dyn Bus) -> SimResult<()> {
        Ok(())
    }
    fn step(
        &mut self,
        _bus: &mut dyn Bus,
        _observers: &[Arc<dyn SimulationObserver>],
        _config: &crate::SimulationConfig,
    ) -> SimResult<()> {
        self.pc += 2;
        Ok(())
    }
    fn set_pc(&mut self, val: u32) {
        self.pc = val;
    }
    fn get_pc(&self) -> u32 {
        self.pc
    }
    fn set_sp(&mut self, _val: u32) {}
    fn set_exception_pending(&mut self, _exception_num: u32) {}
    fn get_register(&self, _id: u8) -> u32 {
        0
    }
    fn set_register(&mut self, _id: u8, _val: u32) {}
    fn snapshot(&self) -> crate::snapshot::CpuSnapshot {
        crate::snapshot::CpuSnapshot::Arm(crate::snapshot::ArmCpuSnapshot {
            registers: vec![0; 16],
            pc: self.pc,
            xpsr: 0,
            primask: false,
            pending_exceptions: 0,
            pending_exceptions_hi: Vec::new(),
            vtor: 0,
        })
    } // Dummy
    fn apply_snapshot(&mut self, _snapshot: &crate::snapshot::CpuSnapshot) {}
    fn get_register_names(&self) -> Vec<String> {
        vec![]
    }

    fn index_of_register(&self, _name: &str) -> Option<u8> {
        None
    }
}

#[test]
fn test_machine_run_cycles() {
    let cpu = MockCpu::default();
    let bus = crate::bus::SystemBus::new();
    let mut machine = Machine::new(cpu, bus);

    assert_eq!(machine.total_cycles, 0);

    // Run 100 steps
    let reason = machine.run(Some(100)).unwrap();
    assert_eq!(reason, StopReason::MaxStepsReached);
    assert_eq!(machine.total_cycles, 100);

    // Run 50 steps
    let reason = machine.run(Some(50)).unwrap();
    assert_eq!(reason, StopReason::MaxStepsReached);
    assert_eq!(machine.total_cycles, 150);
}

#[test]
fn legacy_run_breakpoint_stickiness_allows_one_step_then_rearms() {
    let cpu = MockCpu::default();
    let bus = crate::bus::SystemBus::new();
    let mut machine = Machine::new(cpu, bus);
    machine.set_pc(0x1001);
    machine.add_breakpoint(0x1000);

    let first = machine.run(Some(1)).unwrap();
    assert_eq!(first, StopReason::Breakpoint(0x1001));

    let stepped = machine.run(Some(1)).unwrap();
    assert_eq!(stepped, StopReason::MaxStepsReached);
    assert_eq!(machine.get_pc(), 0x1003);

    machine.set_pc(0x1001);
    let rearmed = machine.run(Some(1)).unwrap();
    assert_eq!(rearmed, StopReason::Breakpoint(0x1001));
}

#[test]
fn machine_run_records_step_profile_counters() {
    let cpu = MockCpu::default();
    let bus = crate::bus::SystemBus::new();
    let mut machine = Machine::new(cpu, bus);
    machine.config.peripheral_tick_interval = 4;

    machine.reset_step_profile();
    let reason = machine.run(Some(10)).unwrap();
    assert_eq!(reason, StopReason::MaxStepsReached);

    let profile = machine.step_profile();
    assert_eq!(profile.cpu_instructions, 10);
    assert_eq!(profile.cpu_batches, 3);
    assert_eq!(profile.peripheral_ticks, 2);
    assert_eq!(profile.peripheral_ticked_entries, 0);
    assert_eq!(profile.bus_tick_entries, 0);
    // 2 peripheral ticks x the number of WALK-ACTIVE peripherals on the default
    // bus. That bus has four (uart1, gpioa, rcc, systick), but `gpioa` is a
    // `GpioPort`, which overrides neither `tick()` nor `tick_elapsed()` and so
    // reports `legacy_tick_active() == false` — it is no longer visited. Hence
    // 3 active entries per tick, not 4.
    //
    // This counter is a profiling diagnostic, not a behavioural assertion: the
    // drop from 8 to 6 is the intended effect of removing a no-op peripheral
    // from the per-cycle walk, and no observable peripheral behaviour changed.
    let expected_legacy_tick_entries = if cfg!(feature = "event-scheduler") {
        4
    } else {
        6
    };
    assert_eq!(profile.legacy_tick_entries, expected_legacy_tick_entries);
}

#[test]
fn step_profile_serializes_the_standardized_counter_contract() {
    let profile = crate::StepProfile {
        cpu_instructions: 1,
        cpu_batches: 2,
        peripheral_ticks: 3,
        peripheral_ticked_entries: 4,
        bus_tick_entries: 5,
        legacy_tick_entries: 6,
    };
    let value = serde_json::to_value(profile).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "cpu_instructions": 1,
            "cpu_batches": 2,
            "peripheral_ticks": 3,
            "peripheral_ticked_entries": 4,
            "bus_tick_entries": 5,
            "legacy_tick_entries": 6,
        })
    );
}

/// A peripheral that reports a fixed byte from the side-effect-free `peek`.
#[derive(Debug)]
struct PeekTag(u8);
impl crate::Peripheral for PeekTag {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(self.0)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }
    fn peek(&self, _offset: u64) -> Option<u8> {
        Some(self.0)
    }
}

/// `Machine::peek` clamps to mapped regions: bytes inside a peripheral window
/// come back `Mapped`, but an address in an unmodeled gap comes back
/// `Unmapped` — never a silent zero, so unmodeled space cannot be mistaken for
/// real data.
#[test]
fn machine_peek_marks_unmapped_gaps() {
    use crate::inspect::PeekByte;

    let cpu = MockCpu::default();
    let mut bus = crate::bus::SystemBus::new();
    let base = 0x4000_0000u64;
    bus.add_peripheral("tag", base, 0x10, None, Box::new(PeekTag(0xAB)));
    let machine = Machine::new(cpu, bus);

    // In-window: mapped, side-effect-free value.
    let mapped = machine.peek(base, 4);
    assert_eq!(
        mapped.bytes,
        vec![PeekByte::Mapped(0xAB); 4],
        "mapped peripheral bytes"
    );

    // Straddle the window edge (0x4000_0000..0x4000_0010): last two bytes fall
    // into the unmodeled gap and must be marked Unmapped, not zero.
    let straddle = machine.peek(base + 0x0E, 4);
    assert_eq!(
        straddle.bytes,
        vec![
            PeekByte::Mapped(0xAB),
            PeekByte::Mapped(0xAB),
            PeekByte::Unmapped,
            PeekByte::Unmapped,
        ],
        "gap past the window is Unmapped, not zero-filled"
    );

    // Wholly unmapped high address.
    let gap = machine.peek(0x9000_0000, 2);
    assert_eq!(gap.bytes, vec![PeekByte::Unmapped, PeekByte::Unmapped]);

    // The lossy raw view (wasm fast path) substitutes 0 for the gap.
    assert_eq!(straddle.to_lossy_bytes(), vec![0xAB, 0xAB, 0x00, 0x00]);
}

/// `Machine::inspect` enumerates peripherals, decodes declarative register
/// schemas, and honors the name filter.
#[test]
fn machine_inspect_enumerates_and_filters() {
    use crate::inspect::InspectOpts;
    use labwired_config::{Access, PeripheralDescriptor, RegisterDescriptor};

    let desc = PeripheralDescriptor {
        peripheral: "DEMO".to_string(),
        version: "1.0".to_string(),
        registers: vec![RegisterDescriptor {
            id: "CTRL".to_string(),
            address_offset: 0,
            size: 32,
            access: Access::ReadWrite,
            reset_value: 0x5,
            fields: vec![labwired_config::FieldDescriptor {
                name: "ENABLE".to_string(),
                bit_range: [0, 0],
                description: None,
            }],
            side_effects: None,
        }],
        interrupts: None,
        timing: None,
    };

    let cpu = MockCpu::default();
    let mut bus = crate::bus::SystemBus::new();
    bus.add_peripheral(
        "demo",
        0x5000_0000,
        0x100,
        None,
        Box::new(crate::peripherals::declarative::GenericPeripheral::new(
            desc,
        )),
    );
    bus.add_peripheral("tag", 0x6000_0000, 0x10, None, Box::new(PeekTag(0xAB)));
    let machine = Machine::new(cpu, bus);

    // Enumerate all: both peripherals we added are present.
    let all = machine.inspect(None, &InspectOpts::default());
    assert!(all.peripherals.iter().any(|p| p.name == "demo"));
    assert!(all.peripherals.iter().any(|p| p.name == "tag"));

    // Filter to one; declarative schema decodes the named register + field.
    let one = machine.inspect(Some("demo"), &InspectOpts::default());
    assert_eq!(one.peripherals.len(), 1);
    let ctrl = &one.peripherals[0].registers[0];
    assert_eq!(ctrl.name, "CTRL");
    assert_eq!(ctrl.value, 0x5);
    assert_eq!(ctrl.fields[0].name, "ENABLE");
    assert_eq!(ctrl.fields[0].value, 1);
}
