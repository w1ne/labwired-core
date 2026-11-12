use labwired_core::bus::SystemBus;
use labwired_core::cpu::riscv::{RiscV, RiscVCoreProfile};
use labwired_core::{Cpu, Machine, SimulationConfig};

fn run_word(profile: RiscVCoreProfile, opcode: u32) -> Machine<RiscV> {
    let mut bus = SystemBus::new();
    bus.flash.data = opcode.to_le_bytes().to_vec();
    let mut cpu = RiscV::new_for(profile);
    cpu.pc = 0;
    cpu.mtvec = 0x100;
    Machine::new(cpu, bus)
}

fn chip(core: &str) -> labwired_config::ChipDescriptor {
    let yaml = format!(
        "name: test-riscv\narch: riscv\ncpu_hz: 1000000\ncore: \"{core}\"\nflash:\n  base: 0x0\n  size: 16KB\nram:\n  base: 0x20000000\n  size: 1KB\nperipherals: []\n"
    );
    serde_yaml::from_str(&yaml).expect("fixture chip must parse")
}

#[test]
fn rv32ec_traps_reserved_registers_and_unavailable_extensions() {
    // ADDI x15, x0, 7 is valid on RV32E. ADDI x16, x0, 1 names a register
    // that does not exist on the E base and must raise illegal instruction.
    const ADDI_X15: u32 = 0x0070_0793;
    const ADDI_X16: u32 = 0x0010_0813;
    let mut bus = SystemBus::new();
    bus.flash.data = [ADDI_X15.to_le_bytes(), ADDI_X16.to_le_bytes()].concat();
    let mut cpu = RiscV::new_for(RiscVCoreProfile::Rv32ec);
    cpu.pc = 0;
    cpu.mtvec = 0x100;
    let mut machine = Machine::new(cpu, bus);

    machine.step().unwrap();
    assert_eq!(machine.cpu.x[15], 7);
    assert_eq!(machine.cpu.pc, 4);

    machine.step().unwrap();
    assert_eq!(machine.cpu.mcause, 2, "x16 is reserved on RV32E");
    assert_eq!(machine.cpu.mepc, 4);
    assert_eq!(machine.cpu.mtval, ADDI_X16);
    assert_eq!(machine.cpu.pc, 0x100);
}

#[test]
fn rv32ec_rejects_compressed_high_registers_and_m_extension() {
    let mut bus = SystemBus::new();
    // C.LI x16, 1 (reserved register on RV32E).
    bus.flash.data = 0x0001_4805u32.to_le_bytes().to_vec();
    let mut cpu = RiscV::new_for(RiscVCoreProfile::Rv32ec);
    cpu.pc = 0;
    cpu.mtvec = 0x80;
    let mut machine = Machine::new(cpu, bus);
    machine.step().unwrap();
    assert_eq!(machine.cpu.mcause, 2);
    assert_eq!(machine.cpu.mtval, 0x4805);
    assert_eq!(machine.cpu.pc, 0x80);

    // MUL x1, x2, x3 is understood by the shared decoder but absent from
    // RV32EC. It must not execute just because another profile implements M.
    let mul_x1_x2_x3 = 0x0231_00B3u32;
    let mut bus = SystemBus::new();
    bus.flash.data = mul_x1_x2_x3.to_le_bytes().to_vec();
    let mut cpu = RiscV::new_for(RiscVCoreProfile::Rv32ec);
    cpu.pc = 0;
    cpu.mtvec = 0x80;
    let mut machine = Machine::new(cpu, bus);
    machine.step().unwrap();
    assert_eq!(machine.cpu.mcause, 2);
    assert_eq!(machine.cpu.mtval, mul_x1_x2_x3);
    assert_eq!(machine.cpu.pc, 0x80);

    // An instruction the shared decoder does not implement is still an
    // architectural illegal-instruction trap on this profile.
    const UNKNOWN: u32 = 0xFFFF_FFFF;
    let mut bus = SystemBus::new();
    bus.flash.data = UNKNOWN.to_le_bytes().to_vec();
    let mut cpu = RiscV::new_for(RiscVCoreProfile::Rv32ec);
    cpu.pc = 0;
    cpu.mtvec = 0x80;
    let mut machine = Machine::new(cpu, bus);
    machine.step().unwrap();
    assert_eq!(machine.cpu.mcause, 2);
    assert_eq!(machine.cpu.mtval, UNKNOWN);
    assert_eq!(machine.cpu.pc, 0x80);
}

#[test]
fn rv32ec_reports_its_misa_extensions() {
    let read_misa = (0x301u32 << 20) | (5 << 7) | (0b010 << 12) | 0x73;
    let mut bus = SystemBus::new();
    bus.flash.data = read_misa.to_le_bytes().to_vec();
    let mut cpu = RiscV::new_for(RiscVCoreProfile::Rv32ec);
    cpu.pc = 0;
    let mut machine = Machine::new(cpu, bus);

    machine.step().unwrap();

    assert_eq!(machine.cpu.x[5], 0x4000_0014); // RV32 + E + C
    assert_eq!(machine.cpu.mcause, 0);
}

#[test]
fn rv32ec_traps_reserved_compressed_encodings_before_effects() {
    // C v2.0 reserves the zero-immediate forms and C.LWSP/C.JR with x0.
    // RV32 shifts with shamt[5]=1 name unsupported custom extensions.
    let opcodes = [
        0x8002u16, // C.JR x0
        0x4002,    // C.LWSP x0, 0(sp)
        0x6101,    // C.ADDI16SP sp, 0
        0x6181,    // C.LUI x3, 0
        0x0000,    // C.ADDI4SPN x8, sp, 0
        0x1082,    // custom C.SLLI, shamt[5]=1
        0x9001,    // custom C.SRLI, shamt[5]=1
        0x9401,    // custom C.SRAI, shamt[5]=1
    ];
    for opcode in opcodes {
        for cached in [false, true] {
            // A nonzero next halfword must never leak into mtval.
            let mut machine = run_word(RiscVCoreProfile::Rv32ec, 0xBEEF_0000 | u32::from(opcode));
            machine.cpu.x[2] = 0xDEAD_0000; // invalid SP catches a mistaken load
            machine.cpu.x[3] = 7;
            let before = machine.cpu.x;
            let config = SimulationConfig {
                decode_cache_enabled: cached,
                ..SimulationConfig::default()
            };
            // Repeat at the same PC to exercise a populated decode cache too.
            for _ in 0..2 {
                machine.cpu.pc = 0;
                machine.cpu.step(&mut machine.bus, &[], &config).unwrap();
                assert_eq!(machine.cpu.mcause, 2, "opcode {opcode:#06x}");
                assert_eq!(machine.cpu.mtval, u32::from(opcode));
                assert_eq!(machine.cpu.mepc, 0);
                assert_eq!(machine.cpu.pc, 0x100);
                assert_eq!(machine.cpu.x, before, "trap must precede register effects");
            }
        }
    }
}

#[test]
fn rv32ec_compressed_breakpoint_traps_instead_of_retiring_a_noop() {
    for profile in [
        RiscVCoreProfile::Rv32ec,
        RiscVCoreProfile::StandardRv32,
        RiscVCoreProfile::Esp32C3,
    ] {
        let mut machine = run_word(profile, 0xBEEF_9002); // C.EBREAK
        machine.cpu.x[1] = 0x40;
        let before = machine.cpu.x;

        machine.step().unwrap();

        assert_eq!(machine.cpu.mcause, 3);
        assert_eq!(machine.cpu.mepc, 0);
        assert_eq!(machine.cpu.pc, 0x100);
        assert_eq!(machine.cpu.x, before);
    }
}

#[test]
fn rv32ec_legal_compressed_jumps_and_hints_still_execute() {
    for (opcode, link) in [(0x8082, 0x40), (0x9082, 2)] {
        let mut machine = run_word(RiscVCoreProfile::Rv32ec, opcode); // C.JR/C.JALR x1
        machine.cpu.x[1] = 0x40;
        machine.step().unwrap();
        assert_eq!(machine.cpu.pc, 0x40);
        assert_eq!(machine.cpu.x[1], link);
        assert_eq!(machine.cpu.mcause, 0);
    }
    // C.NOP/C.ADDI, C.LI, C.LUI and C.SLLI with x0 are legal hints.
    for opcode in [0x0001, 0x0005, 0x4005, 0x6005, 0x0002, 0x0006, 0x8001] {
        let mut machine = run_word(RiscVCoreProfile::Rv32ec, opcode);
        machine.cpu.x[8] = 7;
        let before = machine.cpu.x;
        machine.step().unwrap();
        assert_eq!(machine.cpu.pc, 2, "hint {opcode:#06x}");
        assert_eq!(machine.cpu.x, before);
        assert_eq!(machine.cpu.mcause, 0);
    }
}

#[test]
fn rv32ec_rejects_atomic_and_high_register_store_without_writes() {
    for opcode in [0x0831_20AFu32, 0x0101_2023] {
        // AMOSWAP.W x1,x3,(x2); SW x16,0(x2)
        let mut machine = run_word(RiscVCoreProfile::Rv32ec, opcode);
        machine.cpu.x[2] = machine.bus.ram.base_addr as u32;
        machine.cpu.x[3] = 0xDEAD_BEEF;
        machine.cpu.x[16] = 0xBAD0_BAD0;
        machine.bus.ram.data[..4].copy_from_slice(&0x1234_5678u32.to_le_bytes());
        machine.step().unwrap();
        assert_eq!(machine.cpu.mcause, 2);
        assert_eq!(machine.cpu.mtval, opcode);
        assert_eq!(machine.cpu.pc, 0x100);
        assert_eq!(machine.cpu.x[1], 0);
        assert_eq!(&machine.bus.ram.data[..4], &0x1234_5678u32.to_le_bytes());
    }
}

#[test]
fn rv32ec_does_not_disable_multiply_for_rv32em_profiles() {
    for core in ["rv32em", "rv32emc"] {
        let mut machine = run_word(RiscVCoreProfile::for_chip(&chip(core)), 0x0231_00B3);
        machine.cpu.x[2] = 6;
        machine.cpu.x[3] = 7;
        machine.step().unwrap();
        assert_eq!(machine.cpu.x[1], 42, "{core} must retain M");
        assert_eq!(machine.cpu.pc, 4);
        assert_eq!(machine.cpu.mcause, 0);
    }
}

#[test]
fn rv32ec_csr_immediates_are_not_register_numbers() {
    // CSRRWI x1, mscratch, 31 is legal; the 5-bit immediate is not x31.
    let mut machine = run_word(RiscVCoreProfile::Rv32ec, 0x340F_D0F3);
    machine.step().unwrap();
    assert_eq!(machine.cpu.mscratch, 31);
    assert_eq!(machine.cpu.mcause, 0);
    assert_eq!(machine.cpu.pc, 4);
}

#[test]
fn rv32ec_batch_keeps_warm_illegal_loops_on_the_checked_interpreter() {
    const FLASH: u32 = 0x0800_0000;
    let mut machine = run_word(RiscVCoreProfile::Rv32ec, 0x0000_0013); // NOP
    machine.bus.flash.base_addr = u64::from(FLASH);
    machine.bus.flash.data.resize(0x104, 0);
    machine.bus.flash.data[4..8].copy_from_slice(&0x0018_0813u32.to_le_bytes()); // ADDI x16,x16,1
    machine.bus.flash.data[8..12].copy_from_slice(&0xFFDF_F06Fu32.to_le_bytes()); // J -4
    machine.bus.flash.data[0x100..0x104].copy_from_slice(&0x0000_006Fu32.to_le_bytes());
    machine.cpu.pc = FLASH;
    machine.cpu.mtvec = FLASH + 0x100;
    // Retire a legal NOP in mapped flash to warm the same fetch window as the
    // illegal loop. The first batch must validate x16 even with warm code.
    machine.step().unwrap();
    assert_eq!(machine.cpu.pc, FLASH + 4);
    let config = SimulationConfig {
        decode_cache_enabled: true,
        riscv_jit_enabled: true,
        ..SimulationConfig::default()
    };
    machine
        .cpu
        .step_batch(&mut machine.bus, &[], &config, 64)
        .unwrap();
    assert_eq!(machine.cpu.mcause, 2);
    assert_eq!(machine.cpu.mtval, 0x0018_0813);
    assert_eq!(machine.cpu.mepc, FLASH + 4);
    assert_eq!(machine.cpu.pc, FLASH + 0x100);
    assert_eq!(machine.cpu.x[16], 0);
    #[cfg(feature = "jit")]
    assert!(
        machine.cpu.jit_engine_stats().is_none(),
        "RV32IMC JIT must stay unused"
    );
}
