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
    // DevKitC-1 uses ARDUINO_USB_MODE=1 → Serial is USB-Serial-JTAG, not UART0.
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
    // Hybrid preserve parks under FreeRTOS TCB; without the real S3
    // pxCurrentTCBs base, APP loses windows across vTaskDelay.
    for sym in ["pxCurrentTCBs", "pxCurrentTCB"] {
        if let Some(a) = labwired_loader::resolve_symbol_in_elf(&fw, sym) {
            rom_thunks::PX_CURRENT_TCB_ADDR.with(|s| s.set(Some(a)));
            eprintln!("[diag] pxCurrentTCBs @0x{a:08x}");
            break;
        }
    }
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
    let mut app_uf_logged = false;
    let cross_isr = labwired_loader::resolve_symbol_in_elf(&fw, "esp_crosscore_isr").unwrap_or(0);
    let wrapper = labwired_loader::resolve_symbol_in_elf(&fw, "vPortTaskWrapper").unwrap_or(0);
    let main_task = labwired_loader::resolve_symbol_in_elf(&fw, "main_task").unwrap_or(0);
    let app_main = labwired_loader::resolve_symbol_in_elf(&fw, "app_main").unwrap_or(0);
    let loop_task = labwired_loader::resolve_symbol_in_elf(&fw, "_Z8loopTaskPv").unwrap_or(0);
    let setup_sym = labwired_loader::resolve_symbol_in_elf(&fw, "_Z5setupv").unwrap_or(0);
    let println_sym =
        labwired_loader::resolve_symbol_in_elf(&fw, "_ZN5Print7printlnEPKc").unwrap_or(0);
    // Prefer loader; fall back to nm table (loader may filter non-thunk symbols).
    let sym_or = |name: &str, fb: u32| {
        labwired_loader::resolve_symbol_in_elf(&fw, name)
            .or_else(|| {
                syms.iter()
                    .find(|(_, n)| n.as_str() == name)
                    .map(|(&a, _)| a)
            })
            .unwrap_or(fb)
    };
    let uart_write = sym_or("uartWrite", 0);
    let uart_write_bytes = sym_or("uart_write_bytes", 0);
    let uart_hal_tx = sym_or("uart_hal_write_txfifo", 0);
    let hws_write_buf = sym_or("_ZN14HardwareSerial5writeEPKhj", 0);
    eprintln!(
        "[diag] syms println=0x{println_sym:08x} uartWrite=0x{uart_write:08x} uart_write_bytes=0x{uart_write_bytes:08x} hal_tx=0x{uart_hal_tx:08x} hws_wbuf=0x{hws_write_buf:08x}"
    );
    let mut println_hit = false;
    let mut uw_hit = false;
    let mut uwb_hit = false;
    let mut uhal_hit = 0u32;
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
            eprintln!("[diag] WRAPPER hit @{step} -> {f}");
        }
        if main_task != 0 && pc == main_task {
            eprintln!("[diag] MAIN_TASK @{step}");
        }
        if app_main != 0 && pc == app_main {
            eprintln!("[diag] APP_MAIN @{step}");
        }

        if !app_uf_logged {
            if let Some(app) = machine.cpu_secondary.as_ref() {
                let ap = app.get_pc();
                if (0x403780c0..0x40378100).contains(&ap) {
                    app_uf_logged = true;
                    eprintln!(
                        "[diag] APP_UF pc=0x{ap:08x} a0={:08x} a1={:08x} a9={:08x} pro_pc=0x{pc:08x}",
                        app.get_register(0),
                        app.get_register(1),
                        app.get_register(9),
                    );
                }
            }
        }

        let app_pc_now = machine
            .cpu_secondary
            .as_ref()
            .map(|c| c.get_pc())
            .unwrap_or(0);
        if loop_task != 0 && (pc == loop_task || app_pc_now == loop_task) {
            eprintln!("[diag] LOOPTASK @{step} pro=0x{pc:08x} app=0x{app_pc_now:08x}");
        }
        if setup_sym != 0 && (pc == setup_sym || app_pc_now == setup_sym) {
            eprintln!("[diag] SETUP @{step} pro=0x{pc:08x} app=0x{app_pc_now:08x}");
        }
        if !println_hit && println_sym != 0 && (pc == println_sym || app_pc_now == println_sym) {
            println_hit = true;
            // HardwareSerial @ Serial0 0x3fc9a820: +16 uart_nr, +20 uart*, +72 lock?
            let s0 = 0x3fc9_a820u32;
            let words: Vec<u32> = (0..20)
                .map(|i| machine.bus.read_u32((s0 + i * 4) as u64).unwrap_or(0xdead))
                .collect();
            eprintln!(
                "[diag] PRINTLN @{step} pro=0x{pc:08x} app=0x{app_pc_now:08x} Serial0={words:08x?}"
            );
        }
        if !uw_hit && uart_write != 0 && (pc == uart_write || app_pc_now == uart_write) {
            uw_hit = true;
            eprintln!("[diag] uartWrite @{step}");
        }
        if hws_write_buf != 0 && (pc == hws_write_buf || app_pc_now == hws_write_buf) && !uw_hit {
            eprintln!("[diag] HardwareSerial::write(buf) @{step} app=0x{app_pc_now:08x}");
        }
        if !uwb_hit
            && uart_write_bytes != 0
            && (pc == uart_write_bytes || app_pc_now == uart_write_bytes)
        {
            uwb_hit = true;
            eprintln!("[diag] uart_write_bytes @{step}");
        }
        if uart_hal_tx != 0 && (pc == uart_hal_tx || app_pc_now == uart_hal_tx) {
            uhal_hit += 1;
            if uhal_hit <= 5 {
                eprintln!("[diag] uart_hal_write_txfifo #{uhal_hit} @{step}");
            }
        }
        {
            let u = uart.lock().unwrap();
            if u.windows(8).any(|w| w == b"LW_L0_OK") {
                eprintln!(
                    "[diag] SUCCESS LW_L0_OK @{step} uart={}",
                    String::from_utf8_lossy(&u)
                );
                return;
            }
        }
        if base != last_fn && step > 8000 && (step < 200_000 || step > 170_000) {
            eprintln!("[diag] @{step} -> {f}");
            last_fn = base.to_string();
        } else if base != last_fn {
            last_fn = base.to_string();
        }
        if step % 500_000 == 0 || step == 180_000 {
            let p0 = machine.bus.pending_cpu_irqs(0);
            let p1 = machine.bus.pending_cpu_irqs(1);
            let map57 = machine.bus.read_u32(0x600c_2000 + 57 * 4).unwrap_or(0xdead);
            let map57b = machine
                .bus
                .read_u32(0x600c_2000 + 0x800 + 57 * 4)
                .unwrap_or(0xdead);
            let st_conf = machine.bus.read_u32(0x6002_3000).unwrap_or(0);
            let st_ena = machine.bus.read_u32(0x6002_3064).unwrap_or(0);
            let st_raw = machine.bus.read_u32(0x6002_3068).unwrap_or(0);
            let st_target_conf = machine.bus.read_u32(0x6002_3034).unwrap_or(0);
            // Snapshot unit counters (write UNIT0_OP/UNIT1_OP update bit first)
            let _ = machine.bus.write_u32(0x6002_3004, 1 << 30);
            let _ = machine.bus.write_u32(0x6002_3008, 1 << 30);
            let u0 = machine.bus.read_u32(0x6002_3044).unwrap_or(0);
            let u1 = machine.bus.read_u32(0x6002_304c).unwrap_or(0);
            let real_tgt = machine.bus.read_u32(0x6002_3074).unwrap_or(0);
            let tcb0 = machine.bus.read_u32(0x3fc9_b2b4).unwrap_or(0);
            let tcb1 = machine.bus.read_u32(0x3fc9_b2b8).unwrap_or(0);
            let sch0 = machine.bus.read_u32(0x3fc9_b2dc).unwrap_or(0);
            let sch1 = machine.bus.read_u32(0x3fc9_b2e0).unwrap_or(0);
            let loop_h = machine.bus.read_u32(0x3fc9_a8d0).unwrap_or(0);
            let app_pc = machine
                .cpu_secondary
                .as_ref()
                .map(|c| c.get_pc())
                .unwrap_or(0);
            let app_f = nearest(&syms, app_pc);
            let u = uart.lock().unwrap();
            eprintln!(
                "[diag] hb {step} pro={f} app={app_f} pend=[{p0:08x},{p1:08x}] map57=[{map57:x}/{map57b:x}] st conf={st_conf:08x} tconf={st_target_conf:08x} ena={st_ena:x} raw={st_raw:x} u0={u0} u1={u1} tgt={real_tgt} tcb=[{tcb0:x},{tcb1:x}] sch=[{sch0},{sch1}] loopH={loop_h:x} uart={} isr={isr_hits}",
                u.len()
            );
        }
    }
    let u = uart.lock().unwrap();
    let uart0_status = machine.bus.read_u32(0x6000_001c).unwrap_or(0);
    let uart0_int_raw = machine.bus.read_u32(0x6000_0004).unwrap_or(0);
    let uart0_int_ena = machine.bus.read_u32(0x6000_0008).unwrap_or(0);
    let uart0_clkdiv = machine.bus.read_u32(0x6000_0014).unwrap_or(0);
    let map27_0 = machine.bus.read_u32(0x600c_2000 + 27 * 4).unwrap_or(0);
    let map27_1 = machine
        .bus
        .read_u32(0x600c_2000 + 0x800 + 27 * 4)
        .unwrap_or(0);
    eprintln!(
        "[diag] DONE pc=0x{:08x} {} uart={}\n{:?}\n[diag] UART0 status={uart0_status:08x} raw={uart0_int_raw:08x} ena={uart0_int_ena:08x} clkdiv={uart0_clkdiv:08x} map27=[{map27_0:x}/{map27_1:x}]",
        machine.cpu.get_pc(),
        nearest(&syms, machine.cpu.get_pc()),
        u.len(),
        String::from_utf8_lossy(&u)
    );
}
