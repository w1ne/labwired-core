#![cfg(all(feature = "qemu-arm", not(target_arch = "wasm32")))]

use labwired_core::cpu::CortexA7;
use labwired_core::{Bus, Cpu, DmaRequest, SimResult, SimulationConfig, SimulationError};
use std::cell::Cell;

const ENTRY: u32 = 0x1000;
const DEVICE: u64 = 0x4000_0000;

struct TestBus {
    ram: Vec<u8>,
    config: SimulationConfig,
    reads: Cell<usize>,
    writes: usize,
    value: u32,
    denied_fetch: Option<u64>,
}

impl TestBus {
    fn program(words: &[u32]) -> Self {
        let mut bus = Self {
            ram: vec![0; 0x4000],
            config: SimulationConfig::default(),
            reads: Cell::new(0),
            writes: 0,
            value: 0,
            denied_fetch: None,
        };
        for (i, word) in words.iter().enumerate() {
            bus.ram[ENTRY as usize + i * 4..ENTRY as usize + i * 4 + 4]
                .copy_from_slice(&word.to_le_bytes());
        }
        bus
    }
}

impl Bus for TestBus {
    fn read_u8(&self, addr: u64) -> SimResult<u8> {
        self.ram
            .get(addr as usize)
            .copied()
            .ok_or(SimulationError::MemoryViolation(addr))
    }
    fn write_u8(&mut self, addr: u64, value: u8) -> SimResult<()> {
        *self
            .ram
            .get_mut(addr as usize)
            .ok_or(SimulationError::MemoryViolation(addr))? = value;
        Ok(())
    }
    fn read_u32(&self, addr: u64) -> SimResult<u32> {
        if addr == DEVICE {
            self.reads.set(self.reads.get() + 1);
            return Ok(self.value);
        }
        Ok(u32::from_le_bytes([
            self.read_u8(addr)?,
            self.read_u8(addr + 1)?,
            self.read_u8(addr + 2)?,
            self.read_u8(addr + 3)?,
        ]))
    }
    fn write_u32(&mut self, addr: u64, value: u32) -> SimResult<()> {
        if addr == DEVICE {
            self.writes += 1;
            self.value = value;
            return Ok(());
        }
        for (i, byte) in value.to_le_bytes().iter().enumerate() {
            self.write_u8(addr + i as u64, *byte)?;
        }
        Ok(())
    }
    fn tick_peripherals(&mut self) -> Vec<u32> {
        Vec::new()
    }
    fn execute_dma(&mut self, _: &[DmaRequest]) -> SimResult<()> {
        Ok(())
    }
    fn config(&self) -> &SimulationConfig {
        &self.config
    }
    fn check_fetch_permission(&mut self, address: u64) -> labwired_core::FetchPermission {
        if self.denied_fetch == Some(address) {
            labwired_core::FetchPermission::DeniedUndeliverable
        } else {
            labwired_core::FetchPermission::Allowed
        }
    }
}

#[test]
fn instruction_tail_fetch_permission_is_checked_before_execution() {
    let mut bus = TestBus::program(&[0xe3a00007]);
    bus.denied_fetch = Some(u64::from(ENTRY) + 3);
    let mut cpu = cpu();
    assert!(matches!(
        step(&mut cpu, &mut bus),
        Err(SimulationError::MemoryViolation(_))
    ));
    assert_eq!(cpu.get_pc(), ENTRY);
    assert_eq!(cpu.get_register(0), 0);
}

fn cpu() -> CortexA7 {
    let mut cpu = CortexA7::new().expect("native Cortex-A7");
    cpu.set_pc(ENTRY);
    cpu
}

fn step(cpu: &mut dyn Cpu, bus: &mut TestBus) -> SimResult<()> {
    cpu.step(bus, &[], &SimulationConfig::default())
}

#[test]
fn arm_arithmetic_through_dyn_cpu() {
    let mut cpu: Box<dyn Cpu> = Box::new(cpu());
    let mut bus = TestBus::program(&[0xe3a00007, 0xe2801005]); // mov r0,#7; add r1,r0,#5
    step(&mut *cpu, &mut bus).unwrap();
    assert_eq!(cpu.get_pc(), ENTRY + 4);
    step(&mut *cpu, &mut bus).unwrap();
    assert_eq!(cpu.get_register(1), 12);
    assert_eq!(cpu.index_of_register("pc"), Some(15));
}

#[test]
fn thumb_instruction_at_end_of_mapped_bus() {
    let mut cpu = cpu();
    let mut bus = TestBus::program(&[]);
    bus.ram[0x3ffe..].copy_from_slice(&[0x09, 0x20]); // movs r0,#9
    cpu.set_pc(0x3fff); // odd entry selects Thumb
    step(&mut cpu, &mut bus).unwrap();
    assert_eq!(cpu.get_register(0), 9);
    assert_eq!(cpu.get_pc(), 0x4000);
    assert!(matches!(
        step(&mut cpu, &mut bus),
        Err(SimulationError::MemoryViolation(0x4000))
    ));
}

#[test]
fn thumb_it_blocks_and_existing_itstate_are_refused_without_execution() {
    let mut cpu = cpu();
    let mut bus = TestBus::program(&[]);
    bus.ram[ENTRY as usize..ENTRY as usize + 4].copy_from_slice(&[0x08, 0xbf, 0x01, 0x20]); // it eq; moveq r0,#1
    cpu.set_pc(ENTRY | 1);
    cpu.set_register(16, cpu.get_register(16) | (1 << 30)); // Z
    assert!(matches!(
        step(&mut cpu, &mut bus),
        Err(SimulationError::NotImplemented(_))
    ));
    assert_eq!(cpu.get_pc(), ENTRY);
    assert_eq!(cpu.get_register(0), 0);
    bus.ram[ENTRY as usize..ENTRY as usize + 2].copy_from_slice(&[0x01, 0x20]);
    cpu.set_register(16, cpu.get_register(16) | (8 << 10)); // active ITSTATE
    assert!(matches!(
        step(&mut cpu, &mut bus),
        Err(SimulationError::NotImplemented(_))
    ));
    assert_eq!(cpu.get_pc(), ENTRY);
}

#[test]
fn wait_instructions_are_refused_until_idle_scheduling_is_connected() {
    for opcode in [0xe320f002, 0xe320f003] {
        let mut cpu = cpu();
        let mut bus = TestBus::program(&[opcode]);
        assert!(matches!(
            step(&mut cpu, &mut bus),
            Err(SimulationError::NotImplemented(_))
        ));
        assert_eq!(cpu.get_pc(), ENTRY);
    }
}

#[test]
fn mmio_load_store_preserves_word_width_and_exactly_once_side_effects() {
    let mut cpu = cpu();
    let mut bus = TestBus::program(&[0xe5810000, 0xe5912000, 0xe5913000]); // str/ldr/ldr
    cpu.set_register(0, 0xdeadbeef);
    cpu.set_register(1, DEVICE as u32);
    step(&mut cpu, &mut bus).unwrap();
    step(&mut cpu, &mut bus).unwrap();
    bus.value = 0x12345678; // external device changes between steps
    step(&mut cpu, &mut bus).unwrap();
    assert_eq!(bus.writes, 1);
    assert_eq!(bus.reads.get(), 2);
    assert_eq!(cpu.get_register(2), 0xdeadbeef);
    assert_eq!(cpu.get_register(3), 0x12345678);
}

#[test]
fn bus_faults_propagate_and_callbacks_do_not_keep_old_bus() {
    for opcode in [0xe5910000, 0xe5810000] {
        // unmapped load/store
        let mut cpu = cpu();
        let mut bus = TestBus::program(&[opcode]);
        cpu.set_register(1, 0x5000_0000);
        assert!(matches!(
            step(&mut cpu, &mut bus),
            Err(SimulationError::MemoryViolation(0x5000_0000))
        ));
        drop(bus);
        let mut new_bus = TestBus::program(&[0xe3a00003]);
        cpu.set_pc(ENTRY);
        step(&mut cpu, &mut new_bus).unwrap();
        assert_eq!(cpu.get_register(0), 3);
    }
}

#[test]
fn changed_instruction_bytes_invalidate_native_translation() {
    let mut cpu = cpu();
    let mut bus = TestBus::program(&[0xe3a00001]);
    step(&mut cpu, &mut bus).unwrap();
    bus.write_u32(ENTRY as u64, 0xe3a00002).unwrap();
    cpu.set_pc(ENTRY);
    step(&mut cpu, &mut bus).unwrap();
    assert_eq!(cpu.get_register(0), 2);
}

#[test]
fn idle_core_can_move_between_threads() {
    let mut cpu = cpu();
    let mut bus = TestBus::program(&[0xe3a00005]);
    std::thread::spawn(move || {
        step(&mut cpu, &mut bus).unwrap();
        assert_eq!(cpu.get_register(0), 5);
    })
    .join()
    .unwrap();
}

#[test]
fn unsupported_irq_and_snapshot_restore_fail_explicitly() {
    let mut cpu = cpu();
    let mut bus = TestBus::program(&[0xe3a00001]);
    assert!(cpu.runtime_snapshot().is_none());
    cpu.set_exception_pending(32);
    assert!(matches!(
        step(&mut cpu, &mut bus),
        Err(SimulationError::NotImplemented(_))
    ));
    cpu.reset(&mut bus).unwrap();
    cpu.set_pc(ENTRY);
    let snapshot = cpu.snapshot();
    assert_eq!(serde_json::to_value(&snapshot).unwrap()["type"], "cortex_a");
    cpu.apply_snapshot(&snapshot);
    assert!(matches!(
        step(&mut cpu, &mut bus),
        Err(SimulationError::NotImplemented(_))
    ));
}

#[test]
fn guest_exception_is_an_error_instead_of_a_successful_step() {
    let mut cpu = cpu();
    let mut bus = TestBus::program(&[0xef000000]); // svc #0
    assert!(step(&mut cpu, &mut bus).is_err());
}

#[test]
fn enabling_mmu_is_rejected_until_physical_bus_translation_is_connected() {
    let mut cpu = cpu();
    let mut bus = TestBus::program(&[0xee110f10, 0xe3800001, 0xee010f10]);
    step(&mut cpu, &mut bus).unwrap(); // mrc p15,0,r0,c1,c0,0
    step(&mut cpu, &mut bus).unwrap(); // orr r0,r0,#1
    assert!(matches!(
        step(&mut cpu, &mut bus),
        Err(SimulationError::NotImplemented(_))
    ));
}

#[test]
fn machine_uses_existing_bus_device_and_rejects_incomplete_restore() {
    use labwired_core::bus::SystemBus;
    use labwired_core::peripherals::timer::Timer;
    use labwired_core::Machine;
    let mut bus = SystemBus::new();
    bus.add_peripheral("test_timer", DEVICE, 0x400, None, Box::new(Timer::new()));
    bus.write_u32(ENTRY as u64, 0xe5810000).unwrap(); // str r0,[r1]
    let mut cpu = cpu();
    cpu.set_register(0, 0x1234);
    cpu.set_register(1, DEVICE as u32 + 0x2c); // timer ARR
    let mut machine = Machine::new(Box::new(cpu) as Box<dyn Cpu>, bus);
    machine.step().unwrap();
    assert_eq!(machine.bus.read_u32(DEVICE + 0x2c).unwrap(), 0x1234);
    assert!(matches!(
        machine.apply_snapshot(machine.snapshot()),
        Err(SimulationError::NotImplemented(_))
    ));
}

// A real ELF32 layout with one ARM PT_LOAD segment; no external toolchain is
// needed for these constructor tests.
fn elf(entry: u32, words: &[u32]) -> Vec<u8> {
    let mut bytes = vec![0u8; 0x1000 + words.len() * 4];
    bytes[..7].copy_from_slice(b"\x7fELF\x01\x01\x01");
    for (offset, value) in [(16, 2u16), (18, 40), (40, 52), (42, 32), (44, 1)] {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }
    for (offset, value) in [
        (20, 1u32),
        (24, entry),
        (28, 52),
        (52, 1),
        (56, 0x1000),
        (60, ENTRY),
        (64, ENTRY),
        (68, words.len() as u32 * 4),
        (72, words.len() as u32 * 4),
        (76, 5),
        (80, 0x1000),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    for (i, word) in words.iter().enumerate() {
        bytes[0x1000 + i * 4..0x1004 + i * 4].copy_from_slice(&word.to_le_bytes());
    }
    bytes
}

#[test]
fn elf_entry_is_used_instead_of_a_cortex_m_vector_table() {
    use labwired_core::bus::SystemBus;
    let mut bus = SystemBus::new();
    bus.write_u32(0, 0x20001000).unwrap();
    bus.write_u32(4, 0x20000101).unwrap();
    let mut machine =
        labwired_core::system::cortex_a::load_elf(bus, &elf(ENTRY, &[0xe3a00006])).unwrap();
    assert_eq!(machine.cpu.get_pc(), ENTRY);
    machine.step().unwrap();
    assert_eq!(machine.cpu.get_register(0), 6);
}

#[test]
fn shared_builder_and_world_node_choose_the_embedded_core() {
    use labwired_core::machine::AdvanceRequest;
    use labwired_core::system::builder::{
        build_machine, BlobMap, BootMode, BuildOptions, BuildRequest, FirmwareSource,
    };
    use labwired_core::system::node::{build_node, NodeFirmware};
    let chip: labwired_config::ChipDescriptor =
        serde_yaml::from_str(include_str!("../../../examples/cortex-a7-core/chip.yaml")).unwrap();
    let system: labwired_config::SystemManifest =
        serde_yaml::from_str(include_str!("../../../examples/cortex-a7-core/system.yaml")).unwrap();
    let firmware = elf(ENTRY, &[0xe3a00007, 0xe2801005]);
    let blobs = BlobMap::new();
    let mut built = build_machine(BuildRequest {
        chip: &chip,
        system: &system,
        firmware: FirmwareSource::Elf(&firmware),
        boot: BootMode::FastBoot,
        blobs: &blobs,
        options: BuildOptions::default(),
    })
    .unwrap();
    built.machine.advance(AdvanceRequest::run(Some(2))).unwrap();
    assert_eq!(built.machine.read_core_reg(1), 12);
    assert_eq!(built.machine.get_pc(), ENTRY + 8);
    let mut node = build_node("a7", &chip, &system, NodeFirmware::Elf(firmware)).unwrap();
    node.step().unwrap();
    node.step().unwrap();
    assert_eq!(node.get_register(1), 12);
}
