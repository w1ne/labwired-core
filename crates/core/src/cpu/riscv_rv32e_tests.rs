use super::{RiscV, RiscVCoreProfile};
use crate::bus::SystemBus;
use crate::Machine;

const ADDI_X16_X0_1: u32 = 0x0010_0813;
const ADDI_X5_X0_1: u32 = 0x0010_0293;

fn run_word(profile: RiscVCoreProfile, opcode: u32) -> Machine<RiscV> {
    let mut bus = SystemBus::new();
    bus.flash.data = opcode.to_le_bytes().to_vec();
    let mut cpu = RiscV::new_for(profile);
    cpu.pc = 0;
    cpu.mtvec = 0x100;
    Machine::new(cpu, bus)
}

#[test]
fn rv32e_rejects_addi_destination_x16_before_writing_or_advancing() {
    let mut machine = run_word(RiscVCoreProfile::Rv32e, ADDI_X16_X0_1);

    machine.step().unwrap();

    assert_eq!(machine.cpu.mcause, 2);
    assert_eq!(machine.cpu.mtval, 0);
    assert_eq!(machine.cpu.read_reg(16), 0);
    assert_eq!(machine.cpu.mepc, 0);
    assert_eq!(machine.cpu.pc, 0x100);
}

#[test]
fn standard_rv32_accepts_addi_destination_x16() {
    let mut machine = run_word(RiscVCoreProfile::StandardRv32, ADDI_X16_X0_1);

    machine.step().unwrap();

    assert_eq!(machine.cpu.read_reg(16), 1);
    assert_eq!(machine.cpu.pc, 4);
    assert_eq!(machine.cpu.mcause, 0);
}

#[test]
fn rv32e_accepts_legal_addi_x5() {
    let mut machine = run_word(RiscVCoreProfile::Rv32e, ADDI_X5_X0_1);

    machine.step().unwrap();

    assert_eq!(machine.cpu.read_reg(5), 1);
    assert_eq!(machine.cpu.pc, 4);
    assert_eq!(machine.cpu.mcause, 0);
}

#[test]
fn rv32e_rejects_full_width_compressed_register_fields_above_x15() {
    // C.LWSP x16, 0(sp): the destination field is the full five-bit rd.
    let mut lwsp = run_word(RiscVCoreProfile::Rv32e, 0x0000_4802);
    lwsp.step().unwrap();
    assert_eq!(lwsp.cpu.mcause, 2);
    assert_eq!(lwsp.cpu.pc, 0x100);

    // C.JR x16: the source field is likewise a full five-bit register number.
    let mut jr = run_word(RiscVCoreProfile::Rv32e, 0x0000_8802);
    jr.step().unwrap();
    assert_eq!(jr.cpu.mcause, 2);
    assert_eq!(jr.cpu.pc, 0x100);
}

#[test]
fn rv32e_core_strings_select_the_e_profile_without_falling_through() {
    fn chip(core: &str) -> labwired_config::ChipDescriptor {
        let yaml = format!(
            "name: test-riscv\narch: riscv\ncpu_hz: 1000000\ncore: \"{core}\"\nflash:\n  base: 0x0\n  size: 16KB\nram:\n  base: 0x20000000\n  size: 1KB\nperipherals: []\n"
        );
        serde_yaml::from_str(&yaml).expect("fixture chip must parse")
    }

    for core in ["rv32e", "RV32EC", " rv32emc ", "Rv32Em"] {
        assert_eq!(
            RiscVCoreProfile::for_chip(&chip(core)),
            RiscVCoreProfile::Rv32e,
            "core string {core:?} must select RV32E"
        );
    }
}
