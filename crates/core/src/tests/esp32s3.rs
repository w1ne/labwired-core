use crate::bus::SystemBus;
use crate::cpu::xtensa::Xtensa;
use crate::{Bus, Machine};
use labwired_config::{ChipDescriptor, SystemManifest};
use std::path::PathBuf;

#[test]
fn test_esp32s3_full_smoke() {
    let mut chip_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    chip_path.push("../../configs/chips/esp32s3.yaml");

    let mut system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    system_path.push("../../configs/systems/esp32s3-zero.yaml");

    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|_| panic!("Failed to load chip config at {:?}", chip_path));
    let mut manifest = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|_| panic!("Failed to load system manifest at {:?}", system_path));

    let anchored_chip = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored_chip.to_str().unwrap().to_string();

    let mut bus = SystemBus::from_config(&chip, &manifest).expect("Failed to build bus");

    // Xtensa encoding reference (24-bit, little-endian):
    //   inst[3:0]   = op0   (byte0[3:0])
    //   inst[7:4]   = t     (byte0[7:4])
    //   inst[11:8]  = s     (byte1[3:0])
    //   inst[15:12] = r     (byte1[7:4])
    //   inst[23:16] = imm8  (byte2)
    //
    // MOVI at, imm: op0=2, r=0xA, s=register, t=imm[11:8], imm8=imm[7:0]
    // ADDI at, as, imm: op0=2, r=0xC, t=at, s=as, imm8=imm
    // S8I at, as, imm: op0=2, r=0x4, t=at, s=as, imm8=offset
    // J offset: op0=6, r=0, offset in bits[23:6]

    let load_addr = 0x42000000u64;

    // Helper to encode a 24-bit Xtensa instruction
    fn encode_rri8(op0: u8, r: u8, s: u8, t: u8, imm8: u8) -> [u8; 3] {
        let byte0 = (t << 4) | (op0 & 0x0F);
        let byte1 = (r << 4) | (s & 0x0F);
        let byte2 = imm8;
        [byte0, byte1, byte2]
    }

    let mut pc = load_addr;

    // Instruction 1: MOVI a2, 0x4F ('O')
    // op0=2, r=0xA, s=2(=a2), t=0(imm[11:8]), imm8=0x4F
    let inst = encode_rri8(0x02, 0x0A, 2, 0, 0x4F);
    for (i, &b) in inst.iter().enumerate() {
        bus.write_u8(pc + i as u64, b).unwrap();
    }
    pc += 3;

    // Instruction 2: MOVI a3, 0x4B ('K')
    let inst = encode_rri8(0x02, 0x0A, 3, 0, 0x4B);
    for (i, &b) in inst.iter().enumerate() {
        bus.write_u8(pc + i as u64, b).unwrap();
    }
    pc += 3;

    // Instruction 3: S8I a3, a2, 0 (store 'K' at addr in a2, but a2 is 0x4F not a real address)
    // Skip the store - just test register operations

    // Instruction 3: ADDI a4, a2, 1 (a4 = a2 + 1 = 0x50)
    let inst = encode_rri8(0x02, 0x0C, 2, 4, 1);
    for (i, &b) in inst.iter().enumerate() {
        bus.write_u8(pc + i as u64, b).unwrap();
    }
    pc += 3;

    // Instruction 4: J 0 (jump to self - infinite loop)
    // J: op0=0x06, r=0x00 (J subop), s=don't care, t=don't care
    // offset is encoded in bits [23:6], for offset=0:
    // inst = (0 << 6) | (0 << 12) | (0 << 8) | (0 << 4) | 0x06 = 0x000006
    bus.write_u8(pc, 0x06).unwrap();
    bus.write_u8(pc + 1, 0x00).unwrap();
    bus.write_u8(pc + 2, 0x00).unwrap();
    let j_addr = pc;

    let mut cpu = Xtensa::new();
    cpu.pc = load_addr as u32;

    let mut machine = Machine::new(cpu, bus);

    // Execute several steps
    for _ in 0..20 {
        machine.step().expect("Simulation failed");
    }

    // Verify registers
    assert_eq!(machine.cpu.a[2], 0x4F, "a2 should contain 'O' (0x4F) from MOVI");
    assert_eq!(machine.cpu.a[3], 0x4B, "a3 should contain 'K' (0x4B) from MOVI");
    assert_eq!(machine.cpu.a[4], 0x50, "a4 should contain 0x50 (a2 + 1) from ADDI");

    // Verify CPU is looping at J instruction
    assert_eq!(
        machine.cpu.pc,
        j_addr as u32,
        "PC should be at the J loop instruction"
    );
}

#[test]
fn test_esp32s3_uart_write() {
    let mut chip_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    chip_path.push("../../configs/chips/esp32s3.yaml");

    let mut system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    system_path.push("../../configs/systems/esp32s3-zero.yaml");

    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|_| panic!("Failed to load chip config at {:?}", chip_path));
    let mut manifest = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|_| panic!("Failed to load system manifest at {:?}", system_path));

    let anchored_chip = system_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = anchored_chip.to_str().unwrap().to_string();

    let mut bus = SystemBus::from_config(&chip, &manifest).expect("Failed to build bus");

    fn encode_rri8(op0: u8, r: u8, s: u8, t: u8, imm8: u8) -> [u8; 3] {
        [
            (t << 4) | (op0 & 0x0F),
            (r << 4) | (s & 0x0F),
            imm8,
        ]
    }

    let load_addr = 0x42000000u64;
    let mut pc = load_addr;

    // Build UART address 0x60000000 in a2 using MOVI + SLLI
    // Step 1: MOVI a2, 0x60 → a2 = 0x60
    let inst = encode_rri8(0x02, 0x0A, 2, 0, 0x60);
    for (i, &b) in inst.iter().enumerate() { bus.write_u8(pc + i as u64, b).unwrap(); }
    pc += 3;

    // Step 2: SLLI a2, a2, 24 → a2 = 0x60 << 24 = 0x60000000
    // SLLI is in QRST group (op0=0, op1=1, op2=0 or 1)
    // SLLI: op0=0, op1=0x01, op2=(1-(sa>>4)), r=rd, s=sa[3:0], t=rs
    // For SLLI a2, a2, 24: rd=2, rs=2, sa=24
    // sa=24: 32-sa_encoded where sa_encoded = 32-24 = 8
    // Wait... my decoder does: Slli { rd: r, rs: t, sa: 32 - sa } where sa = s | ((op2 & 1) << 4)
    // So we need to encode sa_encoded such that 32 - sa_encoded = 24, i.e. sa_encoded = 8
    // sa_encoded = s | (op2_bit0 << 4). For sa_encoded=8: s=8, op2_bit0=0 → op2=0x00
    // inst format: op0=0x00, t=rs=2, s=sa[3:0]=8, r=rd=2, op1=0x01, op2=0x00
    // byte0 = (t << 4) | op0 = (2 << 4) | 0 = 0x20
    // byte1 = (r << 4) | s = (2 << 4) | 8 = 0x28
    // byte2 = (op2 << 4) | op1 = (0 << 4) | 1 = 0x01
    bus.write_u8(pc, 0x20).unwrap();
    bus.write_u8(pc + 1, 0x28).unwrap();
    bus.write_u8(pc + 2, 0x01).unwrap();
    pc += 3;

    // Step 3: MOVI a3, 75 ('K')
    let inst = encode_rri8(0x02, 0x0A, 3, 0, 75);
    for (i, &b) in inst.iter().enumerate() { bus.write_u8(pc + i as u64, b).unwrap(); }
    pc += 3;

    // Step 4: S8I a3, a2, 0 → write 'K' to UART0
    let inst = encode_rri8(0x02, 0x04, 2, 3, 0);
    for (i, &b) in inst.iter().enumerate() { bus.write_u8(pc + i as u64, b).unwrap(); }
    pc += 3;

    // Step 5: J 0 (infinite loop)
    bus.write_u8(pc, 0x06).unwrap();
    bus.write_u8(pc + 1, 0x00).unwrap();
    bus.write_u8(pc + 2, 0x00).unwrap();

    let mut cpu = Xtensa::new();
    cpu.pc = load_addr as u32;

    let mut machine = Machine::new(cpu, bus);

    for _ in 0..20 {
        machine.step().expect("Simulation failed");
    }

    // Verify a2 holds the UART base address and a3 holds 'K'
    assert_eq!(machine.cpu.a[2], 0x60000000, "a2 should hold UART base 0x60000000");
    assert_eq!(machine.cpu.a[3], 75, "a3 should hold 'K' (75)");
}
