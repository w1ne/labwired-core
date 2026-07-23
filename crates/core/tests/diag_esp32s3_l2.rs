//! ESP32-S3 L2 RGB/RMT fault diag
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

fn dump_maps(bus: &labwired_core::bus::SystemBus, tag: &str) {
    let mut hits = Vec::new();
    for src in 0u32..99 {
        let c0 = bus.read_u32((0x600c_2000 + src * 4) as u64).unwrap_or(0) & 0x1f;
        let c1 = bus
            .read_u32((0x600c_2000 + 0x800 + src * 4) as u64)
            .unwrap_or(0)
            & 0x1f;
        if c0 != 0 || c1 != 0 {
            hits.push(format!("{src}:{c0}/{c1}"));
        }
    }
    eprintln!("[diag] {tag} maps: {}", hits.join(" "));
}

#[test]
#[ignore]
fn diag_s3_l2() {
    std::env::set_var("LABWIRED_ESP32S3_FASTBOOT", "1");
    let core = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let elf = core.join("validation/arduino-matrix/out/esp32s3/L2_blink_serial/firmware.elf");
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
    {
        use labwired_core::peripherals::esp32s3::usb_serial_jtag::UsbSerialJtag;
        for p in bus.peripherals.iter_mut() {
            if p.name == "usb_serial_jtag" {
                if let Some(any) = p.dev.as_any_mut() {
                    if let Some(jtag) = any.downcast_mut::<UsbSerialJtag>() {
                        jtag.set_sink(Some(uart.clone()), false);
                    }
                }
            }
        }
    }
    if let Ok(mut d) = wiring.dcache_backing.lock() {
        if let Ok(pt) = std::fs::read(core.join(
            "validation/arduino-matrix/out/_pio_work/esp32__L0_serial_boot/.pio/build/matrix/partitions.bin",
        )) {
            let n = pt.len().min(0xC00);
            d[0x8000..0x8000 + n].copy_from_slice(&pt[..n]);
        }
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
    for sym in ["pxCurrentTCBs", "pxCurrentTCB"] {
        if let Some(a) = labwired_loader::resolve_symbol_in_elf(&fw, sym) {
            rom_thunks::PX_CURRENT_TCB_ADDR.with(|s| s.set(Some(a)));
            break;
        }
    }
    if let Some(pc) = labwired_loader::resolve_symbol_in_elf(&fw, "xthal_window_spill_nw") {
        let _ = bus.install_flash_thunk(pc, rom_thunks::xthal_window_spill_thunk);
    }
    for sym in ["__assert_func", "panic_abort", "abort"] {
        if let Some(pc) = labwired_loader::resolve_symbol_in_elf(&fw, sym) {
            let _ = bus.install_flash_thunk(pc, rom_thunks::abort_halt);
        }
    }
    let mut app = labwired_core::cpu::xtensa_lx7::XtensaLx7::new_app_cpu();
    app.set_sp(0x3FCD_8000);
    let mut machine = Machine::new(pro, bus).with_secondary_cpu(app);
    machine.config.batch_mode_enabled = false;

    let sym = |n: &str| {
        syms.iter()
            .find(|(_, s)| s.as_str() == n)
            .map(|(&a, _)| a)
            .unwrap_or(0)
    };
    let rgb = sym("rgbLedWrite");
    let rmt_init = sym("rmtInit");
    let rmt_isr = sym("rmt_tx_default_isr");
    let dig_w = sym("__digitalWrite");
    let xqr = sym("xQueueReceive");
    let mut saw_rgb = false;
    let mut isr_n = 0u32;
    let mut rmt_init_logged = false;
    let mut dw_n = 0u32;
    let mut maps_at_1m = false;
    let mut maps_early = [false; 4];

    dump_maps(&machine.bus, "pre-step");

    for step in 1..=6_000_000u64 {
        match machine.step() {
            Ok(()) => {}
            Err(e) => {
                let pc = machine.cpu.get_pc();
                let app_pc = machine
                    .cpu_secondary
                    .as_ref()
                    .map(|c| c.get_pc())
                    .unwrap_or(0);
                eprintln!(
                    "[diag] ERR @{step} pro={} a0={:08x} a1={:08x} a2={:08x} a3={:08x} app={} a0={:08x} a1={:08x} a2={:08x} a3={:08x} pend=[{:08x},{:08x}] e={e}",
                    nearest(&syms, pc),
                    machine.cpu.get_register(0),
                    machine.cpu.get_register(1),
                    machine.cpu.get_register(2),
                    machine.cpu.get_register(3),
                    nearest(&syms, app_pc),
                    machine
                        .cpu_secondary
                        .as_ref()
                        .map(|c| c.get_register(0))
                        .unwrap_or(0),
                    machine
                        .cpu_secondary
                        .as_ref()
                        .map(|c| c.get_register(1))
                        .unwrap_or(0),
                    machine
                        .cpu_secondary
                        .as_ref()
                        .map(|c| c.get_register(2))
                        .unwrap_or(0),
                    machine
                        .cpu_secondary
                        .as_ref()
                        .map(|c| c.get_register(3))
                        .unwrap_or(0),
                    machine.bus.pending_cpu_irqs(0),
                    machine.bus.pending_cpu_irqs(1),
                );
                dump_maps(&machine.bus, "ERR");
                let raw = machine.bus.read_u32(0x6001_6070).unwrap_or(0);
                let ena = machine.bus.read_u32(0x6001_6078).unwrap_or(0);
                eprintln!("[diag] rmt raw={raw:08x} ena={ena:08x}");
                let u = uart.lock().unwrap();
                eprintln!("[diag] uart={:?}", String::from_utf8_lossy(&u));
                return;
            }
        }
        let pc = machine.cpu.get_pc();
        let app_pc = machine
            .cpu_secondary
            .as_ref()
            .map(|c| c.get_pc())
            .unwrap_or(0);

        // Trace who fills the intmatrix 1k..15k
        if step > 1000 && step < 8000 {
            static mut LAST: String = String::new();
            let f = nearest(&syms, pc);
            let base = f.split('+').next().unwrap_or(&f);
            unsafe {
                if base != LAST {
                    eprintln!("[diag] @{step} -> {f}");
                    LAST = base.to_string();
                }
            }
        }

        if !saw_rgb && rgb != 0 && (pc == rgb || app_pc == rgb) {
            saw_rgb = true;
            eprintln!("[diag] rgb @{step}");
            dump_maps(&machine.bus, "pre-rgb");
        }
        if !rmt_init_logged && rmt_init != 0 && (pc == rmt_init || app_pc == rmt_init) {
            rmt_init_logged = true;
            eprintln!("[diag] rmtInit @{step}");
            dump_maps(&machine.bus, "rmtInit");
        }
        if dig_w != 0 && app_pc == dig_w {
            dw_n += 1;
            if dw_n <= 4 {
                eprintln!("[diag] digitalWrite #{dw_n} @{step}");
            }
        }
        if rmt_isr != 0 && (pc == rmt_isr || app_pc == rmt_isr) {
            isr_n += 1;
            if isr_n <= 10 {
                let raw = machine.bus.read_u32(0x6001_6070).unwrap_or(0);
                let ena = machine.bus.read_u32(0x6001_6078).unwrap_or(0);
                eprintln!(
                    "[diag] RMT_ISR #{isr_n} @{step} pro={} app={} pend=[{:08x},{:08x}] raw={raw:x} ena={ena:x}",
                    nearest(&syms, pc),
                    nearest(&syms, app_pc),
                    machine.bus.pending_cpu_irqs(0),
                    machine.bus.pending_cpu_irqs(1),
                );
            }
        }
        if saw_rgb && xqr != 0 && (pc == xqr || app_pc == xqr) {
            let a2p = machine.cpu.get_register(2);
            let a2a = machine
                .cpu_secondary
                .as_ref()
                .map(|c| c.get_register(2))
                .unwrap_or(0);
            let bad = |a: u32| a != 0 && !(0x3fc8_8000..0x3fd0_0000).contains(&a);
            if bad(a2p) || bad(a2a) {
                eprintln!(
                    "[diag] BAD QRECV @{step} pro={} a2={a2p:08x} app={} a2={a2a:08x}",
                    nearest(&syms, pc),
                    nearest(&syms, app_pc),
                );
            }
        }
        for (i, at) in [1_000u64, 10_000, 50_000, 200_000].iter().enumerate() {
            if !maps_early[i] && step >= *at {
                maps_early[i] = true;
                dump_maps(&machine.bus, &format!("step{at}"));
            }
        }
        if !maps_at_1m && step >= 1_000_000 {
            maps_at_1m = true;
            dump_maps(&machine.bus, "1M");
        }
        if step % 250_000 == 0 {
            let u = uart.lock().unwrap().len();
            eprintln!(
                "[diag] hb {step} pro={} app={} uart={u} pend=[{:08x},{:08x}] isr={isr_n}",
                nearest(&syms, pc),
                nearest(&syms, app_pc),
                machine.bus.pending_cpu_irqs(0),
                machine.bus.pending_cpu_irqs(1),
            );
        }
        {
            let u = uart.lock().unwrap();
            if u.windows(8).any(|w| w == b"LW_L2_OK") {
                eprintln!(
                    "[diag] SUCCESS @{step} uart={:?}",
                    String::from_utf8_lossy(&u)
                );
                return;
            }
        }
    }
    eprintln!(
        "[diag] DONE uart={:?}",
        String::from_utf8_lossy(&uart.lock().unwrap())
    );
}
