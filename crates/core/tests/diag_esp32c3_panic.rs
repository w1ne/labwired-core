//! Diagnostic: ESP32-C3 Arduino L0 panic message
use labwired_core::system::builder::build_system_bus;
use labwired_core::{Bus, Cpu, Machine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[test]
#[ignore]
fn diag_c3_panic_message() {
    let core = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let sys = core.join("validation/arduino-matrix/systems/esp32c3.yaml");
    let elf = core.join("validation/arduino-matrix/out/esp32c3/L0_serial_boot/firmware.elf");
    assert!(elf.exists(), "missing {elf:?}");
    let mut bus = build_system_bus(Some(&sys)).expect("bus");
    let flash = Arc::new(Mutex::new(vec![0xFFu8; 4 * 1024 * 1024]));
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
        Box::new(labwired_core::peripherals::esp32s3::spi_mem_flash::SpiMemFlash::new(flash)),
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
    bus.refresh_peripheral_index();

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
        .map(|m| m.data.clone())
        .unwrap();
    for (dst, bytes) in c3_rom_data_init_writes(&irom) {
        for (i, b) in bytes.iter().enumerate() {
            let _ = machine.bus.write_u8(dst as u64 + i as u64, *b);
        }
    }
    let _ = machine.bus.write_u8(0x3C03_0000, 0xE9);
    // SOC_DRAM_HIGH = 0x3FCE0000 — SP must be below for cache-freeze sanity.
    machine.cpu.set_sp(0x3FCD_FFF0);
    machine.cpu.set_pc(program.entry_point as u32);

    let mut hit_setup = false;
    let mut hit_loop = false;
    for step in 1..=2_000_000u64 {
        match machine.step() {
            Ok(()) => {}
            Err(e) => {
                eprintln!("[diag] err step {step}: {e} pc=0x{:08x}", machine.cpu.get_pc());
                break;
            }
        }
        let pc = machine.cpu.get_pc();
        if !hit_setup && pc == 0x4200_0020 {
            // may not be setup VA
            hit_setup = true;
            eprintln!("[diag] HIT pc near app text @ step {step}");
        }
        if pc == 0x4038_41ae || pc == 0x4038_41c0 {
            let a0 = machine.cpu.get_register(10);
            let sp = machine.cpu.get_register(2);
            eprintln!("[diag] panic pc=0x{pc:08x} a0=0x{a0:08x} sp=0x{sp:08x} step={step}");
            let mut s = Vec::new();
            for i in 0..200u64 {
                let b = machine.bus.read_u8(a0 as u64 + i).unwrap_or(0);
                if b == 0 {
                    break;
                }
                s.push(b);
            }
            eprintln!("[diag] a0 str: {:?}", String::from_utf8_lossy(&s));
            return;
        }
        if step % 200_000 == 0 {
            eprintln!(
                "[diag] heartbeat step={step} pc=0x{pc:08x} sp=0x{:08x}",
                machine.cpu.get_register(2)
            );
        }
    }
    let _ = hit_loop;
    eprintln!(
        "[diag] done pc=0x{:08x} (no panic)",
        machine.cpu.get_pc()
    );
}
