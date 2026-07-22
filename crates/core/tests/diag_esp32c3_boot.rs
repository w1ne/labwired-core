//! Diagnostic: ESP32-C3 Arduino L0 boot progress (mirrors CLI fast-boot wiring)
use labwired_core::system::builder::build_system_bus;
use labwired_core::{Bus, Cpu, Machine};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

fn load_syms(elf: &std::path::Path) -> HashMap<u32, String> {
    let out = std::process::Command::new("nm")
        .arg(elf)
        .output()
        .ok();
    let mut m = HashMap::new();
    if let Some(o) = out {
        for line in String::from_utf8_lossy(&o.stdout).lines() {
            let mut p = line.split_whitespace();
            let Some(addr) = p.next() else { continue };
            let Some(_t) = p.next() else { continue };
            let Some(name) = p.next() else { continue };
            if let Ok(a) = u32::from_str_radix(addr, 16) {
                m.insert(a, name.to_string());
            }
        }
    }
    m
}

fn nearest<'a>(syms: &'a HashMap<u32, String>, pc: u32) -> String {
    let mut best: Option<(u32, &str)> = None;
    for (&a, n) in syms {
        if a <= pc {
            if best.map(|(ba, _)| a > ba).unwrap_or(true) {
                best = Some((a, n));
            }
        }
    }
    match best {
        Some((a, n)) => format!("{n}+{:#x}", pc - a),
        None => format!("???"),
    }
}

#[test]
#[ignore]
fn diag_c3_boot_progress() {
    let core = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let sys = core.join("validation/arduino-matrix/systems/esp32c3.yaml");
    let elf = core.join("validation/arduino-matrix/out/esp32c3/L0_serial_boot/firmware.elf");
    assert!(elf.exists(), "missing {elf:?}");
    let syms = load_syms(&elf);

    let mut bus = build_system_bus(Some(&sys)).expect("bus");
    let mut flash_img = vec![0xFFu8; 4 * 1024 * 1024];
    for p in [
        core.join("validation/arduino-matrix/out/_pio_work/esp32c3__L0_serial_boot/.pio/build/matrix/partitions.bin"),
        core.join("validation/arduino-matrix/out/_pio_work/esp32__L0_serial_boot/.pio/build/matrix/partitions.bin"),
    ] {
        if let Ok(pt) = std::fs::read(&p) {
            let n = pt.len().min(0xC00);
            flash_img[0x8000..0x8000 + n].copy_from_slice(&pt[..n]);
            eprintln!("[diag] seeded partitions {} bytes from {}", n, p.display());
            break;
        }
    }
    let flash = Arc::new(Mutex::new(flash_img));
    bus.add_peripheral(
        "spimem1_flash",
        0x6000_2000,
        0x100,
        None,
        Box::new(labwired_core::peripherals::esp32s3::spi_mem_flash::SpiMemFlash::new(
            flash.clone(),
        )),
    );
    bus.add_peripheral(
        "spimem0_flash",
        0x6000_3000,
        0x100,
        None,
        Box::new(labwired_core::peripherals::esp32s3::spi_mem_flash::SpiMemFlash::new(
            flash.clone(),
        )),
    );
    bus.add_peripheral(
        "rtc_i2c_ana",
        0x6000_E000,
        0x400,
        None,
        Box::new(labwired_core::peripherals::esp32c3::ana_i2c::Esp32c3AnaI2c::new()),
    );
    bus.add_peripheral(
        "extmem_cache",
        0x600C_4000,
        0x400,
        None,
        Box::new(labwired_core::peripherals::esp32c3::cache::Esp32c3Cache::new()),
    );
    bus.add_peripheral(
        "systimer",
        0x6002_3000,
        0x100,
        None,
        Box::new(
            labwired_core::peripherals::esp32s3::systimer::Systimer::new_with_source(
                160_000_000, 37,
            ),
        ),
    );
    bus.add_peripheral(
        "apb_saradc",
        0x6004_0000,
        0x100,
        None,
        Box::new(labwired_core::peripherals::esp32c3::sar_adc::Esp32c3SarAdc::new()),
    );
    use labwired_core::peripherals::esp32s3::flash_xip::{
        Esp32s3MmuTable, FlashXipPeripheral, SharedMmu, MMU_FMT_C3,
    };
    let mmu_table = Arc::new(SharedMmu {
        entries: Mutex::new(vec![MMU_FMT_C3.invalid_bit; 128]),
        generation: AtomicU64::new(1),
    });
    bus.add_peripheral(
        "mmu_table",
        0x600C_5000,
        0x800,
        None,
        Box::new(Esp32s3MmuTable::new(mmu_table.clone())),
    );
    bus.add_peripheral(
        "flash_xip_drom",
        0x3C00_0000,
        0x80_0000,
        None,
        Box::new(FlashXipPeripheral::new_mmu_fmt(
            flash.clone(),
            0x3C00_0000,
            mmu_table,
            MMU_FMT_C3,
        )),
    );
    bus.config.optimized_bus_access = false;
    bus.esp32c3_irq_routing = true;
    bus.refresh_peripheral_index();

    let uart = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart.clone(), false);

    let program = labwired_loader::load_elf(&elf).unwrap();
    let cpu = labwired_core::system::riscv::configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&program).unwrap();

    use labwired_core::boot::esp32c3_rom::{c3_rom_data_init_writes, IROM_BASE};
    let irom = machine
        .bus
        .extra_mem
        .iter()
        .find(|m| m.base_addr == IROM_BASE as u64)
        .map(|m| m.data.clone());
    if let Some(irom) = irom {
        if irom.iter().any(|&b| b != 0) {
            for (dst, bytes) in c3_rom_data_init_writes(&irom) {
                for (i, b) in bytes.iter().enumerate() {
                    let _ = machine.bus.write_u8(dst as u64 + i as u64, *b);
                }
            }
        }
    }

    // Flash sync + factory-based MMU (mirrors CLI)
    {
        const PAGE: usize = 64 * 1024;
        const FACTORY_OFF: usize = 0x1_0000;
        const FACTORY_PAGE: u32 = 1;
        let mut virt_pages: Vec<u32> = Vec::new();
        let irom_len = machine.bus.flash.data.len();
        for page in 0..(irom_len + PAGE - 1) / PAGE {
            if page >= 128 {
                break;
            }
            let start = page * PAGE;
            let end = (start + PAGE).min(irom_len);
            if machine.bus.flash.data[start..end].iter().any(|&b| b != 0) {
                virt_pages.push(page as u32);
            }
        }
        let mut f = flash.lock().unwrap();
        let drom = machine
            .bus
            .extra_mem
            .iter()
            .find(|m| m.base_addr == 0x3C00_0000)
            .map(|m| m.data.clone());
        if let Some(d) = drom {
            for page in 0..(d.len() + PAGE - 1) / PAGE {
                if page >= 128 {
                    break;
                }
                let start = page * PAGE;
                let end = (start + PAGE).min(d.len());
                if !d[start..end].iter().any(|&b| b != 0) {
                    continue;
                }
                if !virt_pages.contains(&(page as u32)) {
                    virt_pages.push(page as u32);
                }
                let dst = FACTORY_OFF + start;
                if dst + (end - start) <= f.len() {
                    f[dst..dst + (end - start)].copy_from_slice(&d[start..end]);
                }
            }
        }
        for page in virt_pages.clone() {
            let start = page as usize * PAGE;
            let end = (start + PAGE).min(irom_len);
            let dst = FACTORY_OFF + start;
            if start < irom_len && dst < f.len() {
                let n = (end - start).min(f.len() - dst);
                for i in 0..n {
                    let b = machine.bus.flash.data[start + i];
                    if b != 0 {
                        f[dst + i] = b;
                    }
                }
            }
        }
        for p in [
            core.join("validation/arduino-matrix/out/_pio_work/esp32c3__L0_serial_boot/.pio/build/matrix/partitions.bin"),
            core.join("validation/arduino-matrix/out/_pio_work/esp32__L0_serial_boot/.pio/build/matrix/partitions.bin"),
        ] {
            if let Ok(pt) = std::fs::read(&p) {
                let n = pt.len().min(0xC00);
                f[0x8000..0x8000 + n].copy_from_slice(&pt[..n]);
                break;
            }
        }
        let magic_off = FACTORY_OFF + 0x30000;
        if f.len() > magic_off {
            f[magic_off] = 0xE9;
        }
        drop(f);
        virt_pages.sort_unstable();
        virt_pages.dedup();
        for vp in &virt_pages {
            let _ = machine
                .bus
                .write_u32(0x600C_5000 + (*vp as u64) * 4, FACTORY_PAGE + *vp);
        }
        eprintln!("[diag] factory MMU virt_pages={virt_pages:?}");
        let b0 = machine.bus.read_u8(0x3C03_0000).ok();
        let b1 = machine.bus.read_u8(0x3C03_0020).ok();
        eprintln!("[diag] xip DROM 0x3C030000={b0:02x?} 0x3C030020={b1:02x?}");
    }

    machine.config.batch_mode_enabled = false;
    // No standard CLINT MTIP on C3 — keep mip.MTIP clear (matches rom-boot).
    machine.cpu.mtimecmp = u64::MAX;
    machine.cpu.set_sp(0x3FCD_C000);
    machine.cpu.set_pc(program.entry_point as u32);
    eprintln!(
        "[diag] entry pc=0x{:08x} sp=0x{:08x}",
        machine.cpu.get_pc(),
        machine.cpu.get_register(2)
    );

    let watch = [
        0x4200_4188u32, // ensure_partitions_loaded
        0x4200_54b6,    // spi_flash_mmap
        0x4200_4474,    // esp_partition_find
        0x4201_49fe,    // esp_mmu_map
        0x4038_6f0a,    // mmu_hal_map_region
        0x4038_708e,    // mmu_hal_vaddr_to_paddr
        0x4202_474c,    // main_task
        0x4200_0a70,    // initArduino
        0x4200_0020,    // setup
        0x4200_006e,    // loop (L2)
        0x4200_0bb0,    // delay
        0x4200_0b2c,    // digitalWrite
        0x4200_12aa,    // rgbLedWrite
        0x4202_5e3e,    // rmt_transmit
        0x4200_226a,    // app_main
        0x4038_6a7c,    // vTaskStartScheduler
        0x4038_52ec,    // vPortYield
        0x4038_417e,    // esp_system_abort
        0x4038_41ae,    // panic path
    ];
    let mut hits = std::collections::HashSet::new();
    let mut last_pcs: std::collections::VecDeque<u32> = std::collections::VecDeque::new();
    let mut dump_mmu = |m: &Machine<_>, label: &str| {
        eprint!("[diag] MMU {label}:");
        for i in 0..16u64 {
            let v = m.bus.read_u32(0x600C_5000 + i * 4).unwrap_or(0xDEAD);
            eprint!(" [{i}]={v:#x}");
        }
        eprintln!();
        // Spot-check FlashXip partition page + appdesc
        let p = m.bus.read_u16(0x3C00_8000).unwrap_or(0);
        let e9 = m.bus.read_u8(0x3C03_0000).unwrap_or(0);
        eprintln!("[diag] xip 0x3C008000={p:#x} 0x3C030000={e9:#x}");
    };

    for step in 1..=3_000_000u64 {
        match machine.step() {
            Ok(()) => {}
            Err(e) => {
                let pc = machine.cpu.get_pc();
                eprintln!(
                    "[diag] ERR step={step} pc=0x{pc:08x} {} : {e}",
                    nearest(&syms, pc)
                );
                dump_mmu(&machine, "on-err");
                break;
            }
        }
        let pc = machine.cpu.get_pc();
        if last_pcs.len() >= 8 {
            last_pcs.pop_front();
        }
        last_pcs.push_back(pc);
        for &w in &watch {
            if pc == w && hits.insert(w) {
                let a0 = machine.cpu.get_register(10);
                let a1 = machine.cpu.get_register(11);
                let a2 = machine.cpu.get_register(12);
                eprintln!(
                    "[diag] HIT 0x{w:08x} {} @ step {step} a0={a0:#x} a1={a1:#x} a2={a2:#x}",
                    nearest(&syms, w)
                );
                if w == 0x4200_54b6 || w == 0x4038_6f0a || w == 0x4038_708e {
                    dump_mmu(&machine, "at-hit");
                }
                if w == 0x4038_417e {
                    // esp_system_abort(const char *detail)
                    let mut s = Vec::new();
                    for i in 0..240u64 {
                        let b = machine.bus.read_u8(a0 as u64 + i).unwrap_or(0);
                        if b == 0 {
                            break;
                        }
                        s.push(b);
                    }
                    eprintln!("[diag] abort msg: {:?}", String::from_utf8_lossy(&s));
                }
            }
        }
        // Log every vaddr_to_paddr call's vaddr (a1) once we pass initArduino
        if pc == 0x4038_708e {
            let a1 = machine.cpu.get_register(11);
            static mut N: u32 = 0;
            unsafe {
                N += 1;
                if N <= 12 {
                    let entry_id = ((a1 >> 16) & 0x7f) as usize;
                    let ent = machine
                        .bus
                        .read_u32(0x600C_5000 + entry_id as u64 * 4)
                        .unwrap_or(0);
                    eprintln!(
                        "[diag] vaddr_to_paddr #{N} vaddr={a1:#x} entry[{entry_id}]={ent:#x}"
                    );
                }
            }
        }
        if step % 200_000 == 0 {
            let u = uart.lock().unwrap();
            let us = String::from_utf8_lossy(&u);
            eprintln!(
                "[diag] hb step={step} pc=0x{pc:08x} {} uart_len={} {:?}",
                nearest(&syms, pc),
                u.len(),
                if us.len() > 160 {
                    &us[us.len() - 160..]
                } else {
                    &us[..]
                }
            );
        }
    }
    let u = uart.lock().unwrap();
    let us = String::from_utf8_lossy(&u);
    eprintln!(
        "[diag] DONE pc=0x{:08x} {} uart_len={} last_pcs={:02x?}",
        machine.cpu.get_pc(),
        nearest(&syms, machine.cpu.get_pc()),
        u.len(),
        last_pcs
    );
    eprintln!("[diag] UART:\n{us}");
}
