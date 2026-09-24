// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// RISC-V and Xtensa RTT attach through `labwired test`.
//
// A hand-built ELF that exports `_SEGGER_RTT` drains. The same image with
// the symbol removed still has the ID in RAM, so the CLI scan drains it.
// An ELF with neither the symbol nor the ID fails `rtt_contains`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

const RISCV_TEXT: u32 = 0x8000_0000;
const RISCV_DATA: u32 = 0x8002_0000;
const XTENSA_TEXT: u32 = 0x4008_0000;
/// DRAM `RamPeripheral` on the classic ESP32 test bus.
const XTENSA_DRAM: u32 = 0x3FFB_0000;
/// First address the no-symbol scan probes (`SystemBus::new` RAM).
const XTENSA_SCAN_RAM: u32 = 0x2000_0000;

const RISCV_CODE: &[u8] = &[0x6f, 0x00, 0x00, 0x00];
const XTENSA_CODE: &[u8] = &[0x06, 0xff, 0xff];

const RISCV_BANNER: &str = "RTT hello from riscv";
const XTENSA_BANNER: &str = "RTT hello from xtensa";

fn rtt_image(base: u32, payload: &[u8]) -> Vec<u8> {
    const BUF: usize = 64;
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
    for (i, (_, addr)) in symbols.iter().enumerate() {
        f.extend_from_slice(&str_off[i].to_le_bytes());
        f.extend_from_slice(&addr.to_le_bytes());
        f.extend_from_slice(&0u32.to_le_bytes());
        f.push(0x11);
        f.push(0);
        f.extend_from_slice(&2u16.to_le_bytes());
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

fn riscv_elf(symbol: bool, with_id: bool) -> Vec<u8> {
    let data = if with_id {
        rtt_image(RISCV_DATA, RISCV_BANNER.as_bytes())
    } else {
        Vec::new()
    };
    let symbols: &[(&str, u32)] = if symbol {
        &[("_SEGGER_RTT", RISCV_DATA)]
    } else {
        &[]
    };
    elf32(
        243, RISCV_TEXT, RISCV_TEXT, RISCV_CODE, RISCV_DATA, &data, symbols,
    )
}

fn xtensa_symbol_elf() -> Vec<u8> {
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

fn xtensa_stripped_elf() -> Vec<u8> {
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

/// ID and up-buffer only in the DRAM `RamPeripheral` (`0x3FFB_0000`). Nothing
/// is loaded into `bus.ram` at `0x2000_0000`, so a scan that ignores
/// `RamPeripheral` windows cannot see this block.
fn xtensa_dram_stripped_elf() -> Vec<u8> {
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

fn xtensa_absent_elf() -> Vec<u8> {
    elf32(94, XTENSA_TEXT, XTENSA_TEXT, XTENSA_CODE, 0, &[], &[])
}

struct RttRun {
    exit_code: Option<i32>,
    status: String,
    rtt: String,
    stdout: String,
    stderr: String,
}

fn run_test(root: &Path, fw: &Path, system: &Path, banner: &str, stop_early: bool) -> RttRun {
    run_test_steps(root, fw, system, banner, stop_early, 200_000)
}

fn run_test_steps(
    root: &Path,
    fw: &Path,
    system: &Path,
    banner: &str,
    stop_early: bool,
    max_steps: u64,
) -> RttRun {
    let limits = if stop_early {
        format!(
            "limits:\n  max_steps: {max_steps}\n  stop_when_assertions_pass: true\n  stop_when_assertions_pass_settle_steps: 0\n"
        )
    } else {
        "limits:\n  max_steps: 20000\n".to_string()
    };
    let script_yaml = format!(
        r#"schema_version: "1.0"
inputs:
  firmware: "{}"
  system: "{}"
{limits}assertions:
  - rtt_contains: "{banner}"
"#,
        fw.display(),
        system.display(),
    );
    let out_dir = labwired_cli::test_support::unique_temp_dir("labwired-rtt-arch");
    std::fs::create_dir_all(&out_dir).expect("create out dir");
    let script_path = out_dir.join("script.yaml");
    std::fs::write(&script_path, script_yaml).expect("write script");

    let output = Command::new(env!("CARGO_BIN_EXE_labwired"))
        .current_dir(root)
        .args([
            "test",
            "--script",
            script_path.to_str().unwrap(),
            "--no-uart-stdout",
            "--output-dir",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .expect("execute labwired");

    let rtt = std::fs::read_to_string(out_dir.join("rtt.log")).unwrap_or_default();
    let result = std::fs::read_to_string(out_dir.join("result.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .unwrap_or(serde_json::Value::Null);
    let status = result["status"].as_str().unwrap_or_default().to_string();
    let run = RttRun {
        exit_code: output.status.code(),
        status,
        rtt,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    };
    let _ = std::fs::remove_dir_all(&out_dir);
    run
}

fn write_elf(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("write elf");
    path
}

fn xtensa_system(dir: &Path) -> PathBuf {
    let path = dir.join("system.yaml");
    std::fs::write(
        &path,
        "name: \"rtt-xtensa\"\nchip: \"esp32\"\nexternal_devices: []\n",
    )
    .expect("write system");
    path
}

#[test]
fn riscv_symbol_drains() {
    let root = repo_root();
    let dir = labwired_cli::test_support::unique_temp_dir("labwired-rtt-riscv-sym");
    std::fs::create_dir_all(&dir).unwrap();
    let fw = write_elf(&dir, "fw.elf", &riscv_elf(true, true));
    let system = root.join("configs/systems/ci-fixture-riscv-uart1.yaml");
    let run = run_test(&root, &fw, &system, RISCV_BANNER, true);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        run.exit_code,
        Some(0),
        "stdout:\n{}\nstderr:\n{}\nrtt:\n{}",
        run.stdout,
        run.stderr,
        run.rtt
    );
    assert!(run.rtt.contains(RISCV_BANNER), "rtt.log:\n{}", run.rtt);
    assert_eq!(run.status, "pass");
}

#[test]
fn xtensa_symbol_drains() {
    let root = repo_root();
    let dir = labwired_cli::test_support::unique_temp_dir("labwired-rtt-xtensa-sym");
    std::fs::create_dir_all(&dir).unwrap();
    let fw = write_elf(&dir, "fw.elf", &xtensa_symbol_elf());
    let system = xtensa_system(&dir);
    let run = run_test(&root, &fw, &system, XTENSA_BANNER, true);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        run.exit_code,
        Some(0),
        "stdout:\n{}\nstderr:\n{}\nrtt:\n{}",
        run.stdout,
        run.stderr,
        run.rtt
    );
    assert!(run.rtt.contains(XTENSA_BANNER), "rtt.log:\n{}", run.rtt);
    assert_eq!(run.status, "pass");
}

#[test]
fn elf_without_symbol_fails_rtt_contains() {
    let root = repo_root();
    let dir = labwired_cli::test_support::unique_temp_dir("labwired-rtt-absent");
    std::fs::create_dir_all(&dir).unwrap();
    let riscv_fw = write_elf(&dir, "rv.elf", &riscv_elf(false, false));
    let xtensa_fw = write_elf(&dir, "xt.elf", &xtensa_absent_elf());
    let riscv_system = root.join("configs/systems/ci-fixture-riscv-uart1.yaml");
    let xtensa_sys = xtensa_system(&dir);
    let riscv = run_test(&root, &riscv_fw, &riscv_system, RISCV_BANNER, false);
    let xtensa = run_test(&root, &xtensa_fw, &xtensa_sys, XTENSA_BANNER, false);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        riscv.exit_code,
        Some(1),
        "riscv should fail rtt_contains\nstdout:\n{}\nstderr:\n{}",
        riscv.stdout,
        riscv.stderr
    );
    assert!(!riscv.rtt.contains(RISCV_BANNER), "{}", riscv.rtt);
    assert_eq!(
        xtensa.exit_code,
        Some(1),
        "xtensa should fail rtt_contains\nstdout:\n{}\nstderr:\n{}",
        xtensa.stdout,
        xtensa.stderr
    );
    assert!(!xtensa.rtt.contains(XTENSA_BANNER), "{}", xtensa.rtt);
}

#[test]
fn stripped_symbol_still_drains_on_the_cli_scan() {
    let root = repo_root();
    let dir = labwired_cli::test_support::unique_temp_dir("labwired-rtt-stripped");
    std::fs::create_dir_all(&dir).unwrap();
    let riscv_fw = write_elf(&dir, "rv.elf", &riscv_elf(false, true));
    let xtensa_fw = write_elf(&dir, "xt.elf", &xtensa_stripped_elf());
    let riscv_system = root.join("configs/systems/ci-fixture-riscv-uart1.yaml");
    let xtensa_sys = xtensa_system(&dir);
    let riscv = run_test(&root, &riscv_fw, &riscv_system, RISCV_BANNER, true);
    let xtensa = run_test(&root, &xtensa_fw, &xtensa_sys, XTENSA_BANNER, true);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        riscv.exit_code,
        Some(0),
        "riscv scan should drain\nstdout:\n{}\nstderr:\n{}\nrtt:\n{}",
        riscv.stdout,
        riscv.stderr,
        riscv.rtt
    );
    assert!(riscv.rtt.contains(RISCV_BANNER), "{}", riscv.rtt);
    assert_eq!(
        xtensa.exit_code,
        Some(0),
        "xtensa scan should drain\nstdout:\n{}\nstderr:\n{}\nrtt:\n{}",
        xtensa.stdout,
        xtensa.stderr,
        xtensa.rtt
    );
    assert!(xtensa.rtt.contains(XTENSA_BANNER), "{}", xtensa.rtt);
}

/// The scan has to walk past `bus.ram` (`0x2000_0000`) and the earlier
/// `RamPeripheral` windows before it reaches DRAM. ~310k steps at the
/// 64-cycle / 64-poll cadence; the cap is above that.
#[test]
fn stripped_xtensa_id_only_in_dram_still_drains_on_the_cli_scan() {
    let root = repo_root();
    let dir = labwired_cli::test_support::unique_temp_dir("labwired-rtt-dram-scan");
    std::fs::create_dir_all(&dir).unwrap();
    let fw = write_elf(&dir, "fw.elf", &xtensa_dram_stripped_elf());
    let system = xtensa_system(&dir);
    let run = run_test_steps(&root, &fw, &system, XTENSA_BANNER, true, 500_000);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        run.exit_code,
        Some(0),
        "DRAM-only scan should drain\nstdout:\n{}\nstderr:\n{}\nrtt:\n{}",
        run.stdout,
        run.stderr,
        run.rtt
    );
    assert!(run.rtt.contains(XTENSA_BANNER), "rtt.log:\n{}", run.rtt);
    assert_eq!(run.status, "pass");
}
