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
        // App image magic: identity window used VA 0x3C03_0000 → off 0x30000;
        // factory+MMU maps that VA via entry 3 → phys page 4 (0x40000).
        for off in [0x30000usize, 0x40000, 0x10000] {
            if d.len() > off {
                d[off] = 0xE9;
            }
        }
    }
    let mut pro = wiring.cpu;
    fast_boot(
        &fw,
        &mut bus,
        &mut pro,
        &BootOpts {
            stack_top_fallback: 0x3FCD_FFF0,
            icache_backing: Some(wiring.icache_backing.clone()),
            dcache_backing: Some(wiring.dcache_backing.clone()),
            factory_flash_base: Some(0x1_0000),
        },
    )
    .unwrap();
    labwired_core::boot::esp32s3::seed_factory_mmu_for_cache2phys(&mut bus, 4, 8);
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
    for sym in ["__assert_func", "panic_abort", "abort"] {
        if let Some(pc) = labwired_loader::resolve_symbol_in_elf(&fw, sym) {
            let _ = bus.install_flash_thunk(pc, rom_thunks::abort_halt);
            eprintln!("[diag] abort_halt @ {sym}=0x{pc:08x}");
        }
    }

    // Real APP_CPU: PRO's start_cpu0 waits on s_system_inited[0]&[1]; APP
    // sets [1] via start_cpu_other_cores → do_system_init_fn (PRID bit13=1).
    let mut app = labwired_core::cpu::xtensa_lx7::XtensaLx7::new_app_cpu();
    app.set_sp(0x3FCD_8000);
    let mut machine = Machine::new(pro, bus).with_secondary_cpu(app);
    machine.config.batch_mode_enabled = false;
    let mut last_fn = String::new();
    let mut isr_hits = 0u64;
    let mut wrapper_hits = 0u64;
    let cross_isr = labwired_loader::resolve_symbol_in_elf(&fw, "esp_crosscore_isr").unwrap_or(0);
    let wrapper = labwired_loader::resolve_symbol_in_elf(&fw, "vPortTaskWrapper").unwrap_or(0);
    let main_task = labwired_loader::resolve_symbol_in_elf(&fw, "main_task").unwrap_or(0);
    let app_main = labwired_loader::resolve_symbol_in_elf(&fw, "app_main").unwrap_or(0);
    for step in 1..=3_000_000u64 {
        match machine.step() {
            Ok(()) => {}
            Err(e) => {
                let pc = machine.cpu.get_pc();
                let a1 = machine.cpu.get_register(1);
                let a0 = machine.cpu.get_register(0);
                let a9 = machine.cpu.get_register(9);
                eprintln!(
                    "[diag] ERR step={step} pc=0x{pc:08x} {}: {e}",
                    nearest(&syms, pc)
                );
                let (app_pc, app_a1, app_h) = match machine.cpu_secondary.as_ref() {
                    Some(c) => (c.get_pc(), c.get_register(1), c.halted),
                    None => (0, 0, true),
                };
                eprintln!(
                    "[diag] pro a0=0x{a0:08x} a1=0x{a1:08x} a9=0x{a9:08x} app_pc=0x{app_pc:08x} app_a1=0x{app_a1:08x} app_halted={app_h}"
                );
                break;
            }
        }
        let pc = machine.cpu.get_pc();
        let f = nearest(&syms, pc);
        let base = f.split('+').next().unwrap_or("");
        if cross_isr != 0 && pc == cross_isr {
            isr_hits += 1;
        }
        if wrapper != 0 && pc == wrapper {
            wrapper_hits += 1;
            eprintln!("[diag] WRAPPER hit @{step} -> {f}");
        }
        if main_task != 0 && pc == main_task {
            eprintln!("[diag] MAIN_TASK @{step}");
        }
        if app_main != 0 && pc == app_main {
            eprintln!("[diag] APP_MAIN @{step}");
        }
        if base != last_fn && step > 8000 && step < 200_000 {
            eprintln!("[diag] @{step} -> {f}");
            last_fn = base.to_string();
        } else if base != last_fn {
            last_fn = base.to_string();
        }
        if step % 500_000 == 0 {
            let from0 = machine.bus.read_u32(0x600c_0030).unwrap_or(0xdead);
            let from1 = machine.bus.read_u32(0x600c_0034).unwrap_or(0xdead);
            let p0 = machine.bus.pending_cpu_irqs(0);
            let p1 = machine.bus.pending_cpu_irqs(1);
            let map79 = machine.bus.read_u32(0x600c_2000 + 79 * 4).unwrap_or(0xdead);
            let map80_c0 = machine.bus.read_u32(0x600c_2000 + 80 * 4).unwrap_or(0xdead);
            let map80_c1 = machine
                .bus
                .read_u32(0x600c_2000 + 0x800 + 80 * 4)
                .unwrap_or(0xdead);
            let map79_c1 = machine
                .bus
                .read_u32(0x600c_2000 + 0x800 + 79 * 4)
                .unwrap_or(0xdead);
            let app_pc = machine
                .cpu_secondary
                .as_ref()
                .map(|c| c.get_pc())
                .unwrap_or(0);
            eprintln!(
                "[diag] hb {step} pc=0x{pc:08x} cycles={} FROM=[{from0:x},{from1:x}] pend=[{p0:08x},{p1:08x}] map79=[{map79:x}/{map79_c1:x}] map80=[{map80_c0:x}/{map80_c1:x}] isr={isr_hits} wrap={wrapper_hits}",
                machine.total_cycles
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
