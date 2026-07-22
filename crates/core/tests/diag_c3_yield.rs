//! Probe ESP32-C3 FreeRTOS first-yield / FROM_CPU IPI path (CLI fast-boot mirror).
use labwired_core::system::builder::build_system_bus;
use labwired_core::{Bus, Cpu, Machine};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

fn load_syms(elf: &std::path::Path) -> HashMap<u32, String> {
    let out = std::process::Command::new("nm").arg(elf).output().ok();
    let mut m = HashMap::new();
    if let Some(o) = out {
        for line in String::from_utf8_lossy(&o.stdout).lines() {
            let mut p = line.split_whitespace();
            let (Some(addr), Some(_t), Some(name)) = (p.next(), p.next(), p.next()) else {
                continue;
            };
            if let Ok(a) = u32::from_str_radix(addr, 16) {
                m.insert(a, name.to_string());
            }
        }
    }
    m
}

fn nearest(syms: &HashMap<u32, String>, pc: u32) -> String {
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
        None => "???".into(),
    }
}

#[test]
#[ignore]
fn diag_c3_first_yield() {
    let core = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let sys = core.join("validation/arduino-matrix/systems/esp32c3.yaml");
    let elf = core.join("validation/arduino-matrix/out/esp32c3/L0_serial_boot/firmware.elf");
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
            eprintln!("[diag] seeded partitions {} from {}", n, p.display());
            break;
        }
    }
    let flash = Arc::new(Mutex::new(flash_img));
    bus.add_peripheral(
        "spimem1_flash",
        0x6000_2000,
        0x100,
        None,
        Box::new(
            labwired_core::peripherals::esp32s3::spi_mem_flash::SpiMemFlash::new(flash.clone()),
        ),
    );
    bus.add_peripheral(
        "spimem0_flash",
        0x6000_3000,
        0x100,
        None,
        Box::new(
            labwired_core::peripherals::esp32s3::spi_mem_flash::SpiMemFlash::new(flash.clone()),
        ),
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
                160_000_000,
                37,
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

    eprintln!(
        "[diag] irq_routing={} external_lines={:#x}",
        bus.esp32c3_irq_routing,
        bus.external_irq_lines()
    );
    // Probe SYSTEM FROM_CPU + INTMATRIX MAP for source 50 (reset defaults)
    eprintln!(
        "[diag] FROM_CPU0={:#x} MAP[50]={:#x} INT_ENABLE={:#x} THRESH={:#x}",
        bus.read_u32(0x600C_0028).unwrap_or(0xDEAD),
        bus.read_u32(0x600C_2000 + 50 * 4).unwrap_or(0xDEAD),
        bus.read_u32(0x600C_2104).unwrap_or(0xDEAD),
        bus.read_u32(0x600C_2194).unwrap_or(0xDEAD),
    );

    let uart = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart.clone(), false);

    let program = labwired_loader::load_elf(&elf).unwrap();
    let mut cpu = labwired_core::system::riscv::configure_riscv(&mut bus);
    cpu.mtimecmp = u64::MAX; // match rom-boot: no CLINT MTIP collision
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&program).unwrap();

    use labwired_core::boot::esp32c3_rom::{c3_rom_data_init_writes, IROM_BASE};
    if let Some(irom) = machine
        .bus
        .extra_mem
        .iter()
        .find(|m| m.base_addr == IROM_BASE as u64)
        .map(|m| m.data.clone())
    {
        if irom.iter().any(|&b| b != 0) {
            for (dst, bytes) in c3_rom_data_init_writes(&irom) {
                for (i, b) in bytes.iter().enumerate() {
                    let _ = machine.bus.write_u8(dst as u64 + i as u64, *b);
                }
            }
        }
    }
    {
        // Mirror CLI: identity-map IROM + DROM pages; seed flash image.
        const PAGE: usize = 64 * 1024;
        let mut mmu_pages: Vec<u32> = Vec::new();
        let irom_len = machine.bus.flash.data.len();
        let irom_pages = (irom_len + PAGE - 1) / PAGE;
        for page in 0..irom_pages.min(128) {
            let start = page * PAGE;
            let end = (start + PAGE).min(irom_len);
            if machine.bus.flash.data[start..end].iter().any(|&b| b != 0) {
                mmu_pages.push(page as u32);
            }
        }
        let mut f = flash.lock().unwrap();
        if let Some(d) = machine
            .bus
            .extra_mem
            .iter()
            .find(|m| m.base_addr == 0x3C00_0000)
            .map(|m| m.data.clone())
        {
            let n = d.len().min(f.len());
            f[..n].copy_from_slice(&d[..n]);
            let pages = (n + PAGE - 1) / PAGE;
            for page in 0..pages.min(128) {
                let start = page * PAGE;
                let end = (start + PAGE).min(n);
                if f[start..end].iter().any(|&b| b != 0) && !mmu_pages.contains(&(page as u32)) {
                    mmu_pages.push(page as u32);
                }
            }
        }
        let n_irom = irom_len.min(f.len());
        for (i, b) in machine.bus.flash.data[..n_irom].iter().enumerate() {
            if f[i] == 0xFF {
                f[i] = *b;
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
        if f.len() > 0x30000 {
            f[0x30000] = 0xE9;
        }
        drop(f);
        mmu_pages.sort_unstable();
        mmu_pages.dedup();
        for page in &mmu_pages {
            let _ = machine
                .bus
                .write_u32(0x600C_5000 + (*page as u64) * 4, *page);
        }
        eprintln!("[diag] MMU pages {:?}", mmu_pages);
    }

    machine.cpu.set_sp(0x3FCD_C000);
    machine.cpu.set_pc(program.entry_point as u32);

    let watch = [
        0x42007160u32, // start_cpu0
        0x420247a8,    // esp_startup_start_app
        0x40386a7c,    // vTaskStartScheduler
        0x4038532a,    // xPortStartScheduler
        0x403852ec,    // vPortYield
        0x40381ba8,    // esp_crosscore_int_send_yield
        0x40381af2,    // esp_crosscore_isr
        0x403852d8,    // vPortYieldFromISR
        0x4202474c,    // main_task
        0x4200226a,    // app_main
        0x42002212,    // loopTask
        0x42000020,    // setup
        0x420071b6,    // start_cpu0 infinite loop
    ];
    let mut hits = std::collections::HashSet::new();
    let mut last_from_cpu = 0u32;
    let mut last_lines = 0u32;

    for step in 1..=2_000_000u64 {
        // Sample FROM_CPU + lines around yield
        let from_cpu = machine.bus.read_u32(0x600C_0028).unwrap_or(0);
        let lines = machine.bus.external_irq_lines();
        if from_cpu != last_from_cpu || lines != last_lines {
            let pc = machine.cpu.get_pc();
            let mstatus = machine.cpu.mstatus;
            eprintln!(
                "[diag] step={step} FROM_CPU={from_cpu:#x} lines={lines:#x} mstatus={mstatus:#x} pc={pc:#x} {}",
                nearest(&syms, pc)
            );
            eprintln!(
                "       MAP[50]={:#x} ENABLE={:#x} THRESH={:#x} PRI[line]={:#x}",
                machine.bus.read_u32(0x600C_2000 + 50 * 4).unwrap_or(0),
                machine.bus.read_u32(0x600C_2104).unwrap_or(0),
                machine.bus.read_u32(0x600C_2194).unwrap_or(0),
                {
                    let line = machine.bus.read_u32(0x600C_2000 + 50 * 4).unwrap_or(0) & 0x1F;
                    if line == 0 {
                        0
                    } else {
                        machine
                            .bus
                            .read_u32(0x600C_2114 + (line as u64) * 4)
                            .unwrap_or(0)
                    }
                }
            );
            last_from_cpu = from_cpu;
            last_lines = lines;
        }

        match machine.step() {
            Ok(()) => {}
            Err(e) => {
                let pc = machine.cpu.get_pc();
                // panic_abort stores details pointer in a0 then hits unimp.
                let a0 = machine.cpu.get_register(10);
                let mut msg = Vec::new();
                for i in 0..200u64 {
                    match machine.bus.read_u8(a0 as u64 + i) {
                        Ok(0) | Err(_) => break,
                        Ok(b) => msg.push(b),
                    }
                }
                eprintln!(
                    "[diag] ERR step={step} pc={pc:#x} {} : {e}\n       a0={a0:#x} panic_msg={:?}",
                    nearest(&syms, pc),
                    String::from_utf8_lossy(&msg)
                );
                // Also dump g_panic_abort_details
                if let Ok(p) = machine.bus.read_u32(0x3fc8_fb88) {
                    let mut msg2 = Vec::new();
                    for i in 0..200u64 {
                        match machine.bus.read_u8(p as u64 + i) {
                            Ok(0) | Err(_) => break,
                            Ok(b) => msg2.push(b),
                        }
                    }
                    eprintln!(
                        "       g_panic_abort_details={p:#x} {:?}",
                        String::from_utf8_lossy(&msg2)
                    );
                }
                break;
            }
        }
        let pc = machine.cpu.get_pc();
        for &w in &watch {
            if pc == w && hits.insert(w) {
                eprintln!("[diag] HIT {w:#x} {} @ step {step}", nearest(&syms, w));
            }
        }
        if step % 200_000 == 0 {
            let u = uart.lock().unwrap();
            eprintln!(
                "[diag] hb step={step} pc={pc:#x} {} uart={} lines={:#x}",
                nearest(&syms, pc),
                u.len(),
                machine.bus.external_irq_lines()
            );
        }
        // Stop if stuck in start_cpu0 spin for a while after hitting it
        if hits.contains(&0x420071b6) && step > 50_000 {
            eprintln!("[diag] stuck in start_cpu0 spin; abort");
            break;
        }
    }
    let u = uart.lock().unwrap();
    eprintln!(
        "[diag] DONE pc={:#x} {} uart_len={} uart={:?}",
        machine.cpu.get_pc(),
        nearest(&syms, machine.cpu.get_pc()),
        u.len(),
        String::from_utf8_lossy(&u)
    );
}
