//! Symbol-only RTT attach on the RISC-V and Xtensa wasm constructors.
//!
//! An ELF that exports `_SEGGER_RTT` drains. The same image with the symbol
//! stripped still has the control block in RAM, which the CLI scan would
//! find; wasm must not attach and must not drain.

use super::WasmSimulator;

const RISCV_CHIP: &str = include_str!("../../../configs/chips/ci-fixture-riscv.yaml");
const XTENSA_CHIP: &str = include_str!("../../../configs/chips/esp32.yaml");

const RISCV_SYSTEM: &str = r#"
name: "rtt-riscv"
chip: "ci-fixture-riscv"
external_devices: []
"#;

const XTENSA_SYSTEM: &str = r#"
name: "rtt-xtensa"
chip: "esp32"
external_devices: []
"#;

const RISCV_TEXT: u32 = 0x8000_0000;
const RISCV_DATA: u32 = 0x8002_0000;
/// Classic ESP32 IRAM. The self-jump lives here.
const XTENSA_TEXT: u32 = 0x4008_0000;
/// Inside the DRAM `RamPeripheral` (`0x3FFA_E000`, 200 KiB), not `bus.ram`.
const XTENSA_DRAM: u32 = 0x3FFB_0000;
/// Head of `SystemBus::new`'s RAM. The CLI scan probes this address first.
const XTENSA_SCAN_RAM: u32 = 0x2000_0000;

const RISCV_CODE: &[u8] = &[0x6f, 0x00, 0x00, 0x00]; // jal x0, 0
const XTENSA_CODE: &[u8] = &[0x06, 0xff, 0xff]; // j .

const RISCV_BANNER: &str = "RTT hello from riscv";
const XTENSA_BANNER: &str = "RTT hello from xtensa";

fn rtt_image(base: u32, payload: &[u8]) -> Vec<u8> {
    const BUF: usize = 64;
    assert!(payload.len() < BUF);
    let mut bytes = vec![0u8; 0x30 + BUF];
    bytes[..16].copy_from_slice(b"SEGGER RTT\0\0\0\0\0\0");
    bytes[0x10..0x14].copy_from_slice(&1u32.to_le_bytes());
    let buf = base + 0x30;
    bytes[0x1C..0x20].copy_from_slice(&buf.to_le_bytes());
    bytes[0x20..0x24].copy_from_slice(&(BUF as u32).to_le_bytes());
    bytes[0x24..0x28].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    bytes[0x30..0x30 + payload.len()].copy_from_slice(payload);
    bytes
}

fn pad_to(buf: &mut Vec<u8>, off: usize) {
    if buf.len() < off {
        buf.resize(off, 0);
    }
}

fn align4(n: usize) -> usize {
    (n + 3) & !3
}

/// ELF32 LE, ET_EXEC. `symbols` are `(name, address)` in the data segment.
/// An empty `data` emits no data segment and no `_SEGGER_RTT`.
fn elf32(
    e_machine: u16,
    entry: u32,
    text_addr: u32,
    text: &[u8],
    data_addr: u32,
    data: &[u8],
    symbols: &[(&str, u32)],
) -> Vec<u8> {
    let has_data = !data.is_empty();
    let nph = if has_data { 2 } else { 1 };

    let mut shstr = vec![0u8];
    let mut shstr_off = Vec::new();
    for name in [".text", ".data", ".shstrtab", ".strtab", ".symtab"] {
        if name == ".data" && !has_data {
            shstr_off.push(0);
            continue;
        }
        shstr_off.push(shstr.len() as u32);
        shstr.extend_from_slice(name.as_bytes());
        shstr.push(0);
    }
    let mut strtab = vec![0u8];
    let mut str_off = Vec::new();
    for (name, _) in symbols {
        str_off.push(strtab.len() as u32);
        strtab.extend_from_slice(name.as_bytes());
        strtab.push(0);
    }

    let ph_off = 52usize;
    let text_off = align4(ph_off + 32 * nph);
    let data_off = align4(text_off + text.len());
    let after_loads = if has_data {
        data_off + data.len()
    } else {
        text_off + text.len()
    };
    let shstr_file = align4(after_loads);
    let str_file = align4(shstr_file + shstr.len());
    let sym_count = 1 + symbols.len();
    let sym_file = align4(str_file + strtab.len());
    let shoff = align4(sym_file + 16 * sym_count);
    let shnum = if has_data { 6 } else { 5 };
    let shstrndx = if has_data { 3u16 } else { 2 };

    let mut f = Vec::new();
    f.extend_from_slice(&[0x7f, b'E', b'L', b'F', 1, 1, 1, 0]);
    f.extend_from_slice(&[0u8; 8]);
    f.extend_from_slice(&2u16.to_le_bytes());
    f.extend_from_slice(&e_machine.to_le_bytes());
    f.extend_from_slice(&1u32.to_le_bytes());
    f.extend_from_slice(&entry.to_le_bytes());
    f.extend_from_slice(&(ph_off as u32).to_le_bytes());
    f.extend_from_slice(&(shoff as u32).to_le_bytes());
    f.extend_from_slice(&0u32.to_le_bytes());
    f.extend_from_slice(&52u16.to_le_bytes());
    f.extend_from_slice(&32u16.to_le_bytes());
    f.extend_from_slice(&(nph as u16).to_le_bytes());
    f.extend_from_slice(&40u16.to_le_bytes());
    f.extend_from_slice(&(shnum as u16).to_le_bytes());
    f.extend_from_slice(&shstrndx.to_le_bytes());

    let phdr = |buf: &mut Vec<u8>, offset: usize, addr: u32, len: usize, flags: u32| {
        buf.extend_from_slice(&1u32.to_le_bytes());
        buf.extend_from_slice(&(offset as u32).to_le_bytes());
        buf.extend_from_slice(&addr.to_le_bytes());
        buf.extend_from_slice(&addr.to_le_bytes());
        buf.extend_from_slice(&(len as u32).to_le_bytes());
        buf.extend_from_slice(&(len as u32).to_le_bytes());
        buf.extend_from_slice(&flags.to_le_bytes());
        buf.extend_from_slice(&1u32.to_le_bytes());
    };
    phdr(&mut f, text_off, text_addr, text.len(), 5);
    if has_data {
        phdr(&mut f, data_off, data_addr, data.len(), 6);
    }

    pad_to(&mut f, text_off);
    f.extend_from_slice(text);
    if has_data {
        pad_to(&mut f, data_off);
        f.extend_from_slice(data);
    }
    pad_to(&mut f, shstr_file);
    f.extend_from_slice(&shstr);
    pad_to(&mut f, str_file);
    f.extend_from_slice(&strtab);
    pad_to(&mut f, sym_file);
    f.extend_from_slice(&[0u8; 16]);
    let data_shndx: u16 = 2;
    for (i, (_, addr)) in symbols.iter().enumerate() {
        f.extend_from_slice(&str_off[i].to_le_bytes());
        f.extend_from_slice(&addr.to_le_bytes());
        f.extend_from_slice(&0u32.to_le_bytes());
        f.push(0x11); // STB_GLOBAL | STT_OBJECT
        f.push(0);
        f.extend_from_slice(&data_shndx.to_le_bytes());
    }

    pad_to(&mut f, shoff);
    let she = |buf: &mut Vec<u8>,
               name: u32,
               ty: u32,
               flags: u32,
               addr: u32,
               off: u32,
               size: u32,
               link: u32,
               info: u32,
               entsize: u32| {
        buf.extend_from_slice(&name.to_le_bytes());
        buf.extend_from_slice(&ty.to_le_bytes());
        buf.extend_from_slice(&flags.to_le_bytes());
        buf.extend_from_slice(&addr.to_le_bytes());
        buf.extend_from_slice(&off.to_le_bytes());
        buf.extend_from_slice(&size.to_le_bytes());
        buf.extend_from_slice(&link.to_le_bytes());
        buf.extend_from_slice(&info.to_le_bytes());
        buf.extend_from_slice(&4u32.to_le_bytes());
        buf.extend_from_slice(&entsize.to_le_bytes());
    };
    she(&mut f, 0, 0, 0, 0, 0, 0, 0, 0, 0);
    she(
        &mut f,
        shstr_off[0],
        1,
        6,
        text_addr,
        text_off as u32,
        text.len() as u32,
        0,
        0,
        0,
    );
    if has_data {
        she(
            &mut f,
            shstr_off[1],
            1,
            3,
            data_addr,
            data_off as u32,
            data.len() as u32,
            0,
            0,
            0,
        );
    }
    let str_link = if has_data { 4 } else { 3 };
    she(
        &mut f,
        shstr_off[2],
        3,
        0,
        0,
        shstr_file as u32,
        shstr.len() as u32,
        0,
        0,
        0,
    );
    she(
        &mut f,
        shstr_off[3],
        3,
        0,
        0,
        str_file as u32,
        strtab.len() as u32,
        0,
        0,
        0,
    );
    she(
        &mut f,
        shstr_off[4],
        2,
        0,
        0,
        sym_file as u32,
        (16 * sym_count) as u32,
        str_link,
        1,
        16,
    );
    f
}

fn riscv_with_symbol() -> Vec<u8> {
    let data = rtt_image(RISCV_DATA, RISCV_BANNER.as_bytes());
    elf32(
        243,
        RISCV_TEXT,
        RISCV_TEXT,
        RISCV_CODE,
        RISCV_DATA,
        &data,
        &[("_SEGGER_RTT", RISCV_DATA)],
    )
}

fn riscv_stripped() -> Vec<u8> {
    let data = rtt_image(RISCV_DATA, RISCV_BANNER.as_bytes());
    elf32(
        243,
        RISCV_TEXT,
        RISCV_TEXT,
        RISCV_CODE,
        RISCV_DATA,
        &data,
        &[],
    )
}

fn riscv_no_symbol() -> Vec<u8> {
    elf32(243, RISCV_TEXT, RISCV_TEXT, RISCV_CODE, 0, &[], &[])
}

fn xtensa_with_symbol() -> Vec<u8> {
    let data = rtt_image(XTENSA_DRAM, XTENSA_BANNER.as_bytes());
    elf32(
        94,
        XTENSA_TEXT,
        XTENSA_TEXT,
        XTENSA_CODE,
        XTENSA_DRAM,
        &data,
        &[("_SEGGER_RTT", XTENSA_DRAM)],
    )
}

fn xtensa_stripped() -> Vec<u8> {
    let data = rtt_image(XTENSA_SCAN_RAM, XTENSA_BANNER.as_bytes());
    elf32(
        94,
        XTENSA_TEXT,
        XTENSA_TEXT,
        XTENSA_CODE,
        XTENSA_SCAN_RAM,
        &data,
        &[],
    )
}

/// Same banner, but the control block and its buffer sit only in DRAM.
fn xtensa_dram_stripped() -> Vec<u8> {
    let data = rtt_image(XTENSA_DRAM, XTENSA_BANNER.as_bytes());
    elf32(
        94,
        XTENSA_TEXT,
        XTENSA_TEXT,
        XTENSA_CODE,
        XTENSA_DRAM,
        &data,
        &[],
    )
}

fn chip_and_manifest(
    chip_yaml: &str,
    system_yaml: &str,
) -> (
    labwired_config::ChipDescriptor,
    labwired_config::SystemManifest,
) {
    (
        serde_yaml::from_str(chip_yaml).expect("chip yaml"),
        serde_yaml::from_str(system_yaml).expect("system yaml"),
    )
}

/// Call the arch constructor directly. `new_from_config` parses blobs with
/// `JsValue::is_null`, which is a wasm import and panics on the native test
/// runner. The RISC-V and Xtensa ELF constructors are what this PR attaches.
fn open_riscv(fw: &[u8]) -> WasmSimulator {
    let (chip, manifest) = chip_and_manifest(RISCV_CHIP, RISCV_SYSTEM);
    let blobs = std::collections::HashMap::new();
    WasmSimulator::new_from_config_riscv(&chip, &manifest, fw, &blobs)
        .unwrap_or_else(|_| panic!("new_from_config_riscv failed"))
}

fn open_xtensa(fw: &[u8]) -> WasmSimulator {
    let (_chip, manifest) = chip_and_manifest(XTENSA_CHIP, XTENSA_SYSTEM);
    WasmSimulator::new_from_config_xtensa_esp32(&manifest, fw)
        .unwrap_or_else(|_| panic!("new_from_config_xtensa_esp32 failed"))
}

fn drain_until(sim: &mut WasmSimulator, token: &[u8]) -> Vec<u8> {
    let mut got = Vec::new();
    for _ in 0..40 {
        sim.step_batch(5_000)
            .unwrap_or_else(|e| panic!("step_batch: {e:?}"));
        got.extend(sim.drain_rtt_output().expect("drain"));
        if got.windows(token.len()).any(|w| w == token) {
            break;
        }
    }
    got
}

#[test]
fn symbol_resolves_only_when_exported() {
    let with = riscv_with_symbol();
    assert_eq!(
        labwired_loader::resolve_symbol_in_elf(&with, "_SEGGER_RTT"),
        Some(RISCV_DATA)
    );
    assert_eq!(
        labwired_loader::resolve_symbol_in_elf(&riscv_stripped(), "_SEGGER_RTT"),
        None
    );
    assert_eq!(
        labwired_loader::resolve_symbol_in_elf(&xtensa_with_symbol(), "_SEGGER_RTT"),
        Some(XTENSA_DRAM)
    );
}

#[test]
fn riscv_symbol_drains_on_wasm() {
    let mut sim = open_riscv(&riscv_with_symbol());
    assert!(sim.rtt_attached().unwrap());
    let got = drain_until(&mut sim, RISCV_BANNER.as_bytes());
    let text = String::from_utf8_lossy(&got);
    assert!(
        text.contains(RISCV_BANNER),
        "expected drain of {RISCV_BANNER:?}, got {text:?}"
    );
}

#[test]
fn xtensa_symbol_drains_on_wasm() {
    let mut sim = open_xtensa(&xtensa_with_symbol());
    assert!(sim.rtt_attached().unwrap());
    let got = drain_until(&mut sim, XTENSA_BANNER.as_bytes());
    let text = String::from_utf8_lossy(&got);
    assert!(
        text.contains(XTENSA_BANNER),
        "expected drain of {XTENSA_BANNER:?}, got {text:?}"
    );
}

#[test]
fn stripped_symbol_does_not_drain_on_wasm() {
    for (label, mut sim) in [
        ("riscv", open_riscv(&riscv_stripped())),
        ("xtensa", open_xtensa(&xtensa_stripped())),
        ("xtensa-dram", open_xtensa(&xtensa_dram_stripped())),
    ] {
        assert!(
            !sim.rtt_attached().unwrap(),
            "{label}: stripped ELF must not attach"
        );
        sim.step_batch(20_000)
            .unwrap_or_else(|e| panic!("{label} step: {e:?}"));
        let got = sim.drain_rtt_output().expect("drain");
        assert!(
            got.is_empty(),
            "{label}: wasm scanned RAM and drained {got:?}"
        );
    }
}

#[test]
fn elf_without_the_symbol_or_the_id_does_not_attach() {
    let mut sim = open_riscv(&riscv_no_symbol());
    assert!(!sim.rtt_attached().unwrap());
    sim.step_batch(5_000).unwrap_or_else(|e| panic!("{e:?}"));
    assert!(sim.drain_rtt_output().unwrap().is_empty());
}
