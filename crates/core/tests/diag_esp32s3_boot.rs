//! Diagnostic: ESP32-S3 Arduino L0
use labwired_core::boot::esp32s3::{fast_boot, BootOpts};
use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3Opts};
use labwired_core::{Bus, Cpu, Machine};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn nearest(syms: &HashMap<u32, String>, pc: u32) -> String {
    let mut best = None;
    for (&a, n) in syms {
        if a <= pc && best.map(|(b, _)| a > b).unwrap_or(true) {
            best = Some((a, n.as_str()));
        }
    }
    match best {
        Some((a, n)) => format!("{n}+{:#x}", pc - a),
        None => format!("0x{pc:08x}"),
    }
}

#[test]
#[ignore]
fn diag_s3_boot() {
    std::env::set_var("LABWIRED_ESP32S3_FASTBOOT", "1");
    let core = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let elf = core.join("validation/arduino-matrix/out/esp32s3/L0_serial_boot/firmware.elf");
    let fw = std::fs::read(&elf).unwrap();
    let mut syms = HashMap::new();
    if let Ok(o) = std::process::Command::new("nm").arg(&elf).output() {
        for line in String::from_utf8_lossy(&o.stdout).lines() {
            let mut p = line.split_whitespace();
            let Some(a) = p.next() else { continue };
            let Some(_) = p.next() else { continue };
            let Some(n) = p.next() else { continue };
            if let Ok(a) = u32::from_str_radix(a, 16) {
                syms.insert(a, n.to_string());
            }
        }
    }
    let mut bus = labwired_core::bus::SystemBus::new();
    let wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
    let uart = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart.clone(), false);
    if let Ok(mut d) = wiring.dcache_backing.lock() {
        if let Ok(pt) = std::fs::read(core.join(
            "validation/arduino-matrix/out/_pio_work/esp32__L0_serial_boot/.pio/build/matrix/partitions.bin",
        )) {
            let n = pt.len().min(0xC00);
            d[0x8000..0x8000 + n].copy_from_slice(&pt[..n]);
        }
        if d.len() > 0x30000 {
            d[0x30000] = 0xE9;
        }
    }
    let mut pro = wiring.cpu;
    fast_boot(
        &fw,
        &mut bus,
        &mut pro,
        &BootOpts {
            stack_top_fallback: 0x3FCD_FFF0,
            icache_backing: Some(wiring.icache_backing),
            dcache_backing: Some(wiring.dcache_backing),
        },
    )
    .unwrap();
    use labwired_core::peripherals::esp_xtensa_common::rom_thunks;
    let mut flags = Vec::new();
    for sym in ["s_cpu_up", "s_cpu_inited"] {
        if let Some(a) = labwired_loader::resolve_symbol_in_elf(&fw, sym) {
            flags.push(a);
            flags.push(a + 1);
        }
    }
    rom_thunks::set_appcpu_up_flags(flags);
    if let Some(pc) = labwired_loader::resolve_symbol_in_elf(&fw, "xthal_window_spill_nw") {
        let _ = bus.install_flash_thunk(pc, rom_thunks::xthal_window_spill_thunk);
    }

    let mut machine = Machine::new(pro, bus);
    machine.config.batch_mode_enabled = false;
    let mut last_fn = String::new();
    for step in 1..=8_000_000u64 {
        match machine.step() {
            Ok(()) => {}
            Err(e) => {
                eprintln!(
                    "[diag] ERR step={step} pc=0x{:08x} {}: {e}",
                    machine.cpu.get_pc(),
                    nearest(&syms, machine.cpu.get_pc())
                );
                break;
            }
        }
        let pc = machine.cpu.get_pc();
        let f = nearest(&syms, pc);
        let base = f.split('+').next().unwrap_or("");
        if base != last_fn && step > 8000 {
            eprintln!("[diag] @{step} -> {f}");
            last_fn = base.to_string();
        }
        if step == 39950 || step % 1_000_000 == 0 {
            let cs = machine.bus.read_u32(0x600c_4130).unwrap_or(0xdead);
            eprintln!("[diag] CACHE_STATE=0x{cs:08x}");
        }
        if step % 1_000_000 == 0 {
            let u = uart.lock().unwrap();
            eprintln!(
                "[diag] hb {step} pc=0x{pc:08x} {f} uart={} {:?}",
                u.len(),
                String::from_utf8_lossy(&u)
            );
        }
    }
    let u = uart.lock().unwrap();
    eprintln!(
        "[diag] DONE pc=0x{:08x} {} uart={}\n{:?}",
        machine.cpu.get_pc(),
        nearest(&syms, machine.cpu.get_pc()),
        u.len(),
        String::from_utf8_lossy(&u)
    );
}
