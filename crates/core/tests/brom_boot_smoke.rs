// Phase 1 sanity check for the chip-model roadmap (#105):
// load the real Espressif ESP32 BROM ELF into the simulator's memory map
// and try to execute from the reset vector at 0x40000400.
//
// Expected outcome: BROM will run a small number of instructions then
// stall when it touches a peripheral we haven't modeled (TIMG / RTC_CNTL
// / SYSREG / DPORT, the Phase 2 work). Logging the stall point gives
// Phase 2 its concrete acceptance criteria.
//
// This is intentionally a smoke test — passing means "BROM was loaded
// + executed at least one instruction without crashing the simulator".
// Failing means the loader or memory map is broken before the BROM
// even starts running.

use labwired_core::bus::SystemBus;
use labwired_core::system::xtensa::configure_xtensa_esp32;
use labwired_core::{Bus, Cpu, Machine};
use std::path::PathBuf;

#[test]
fn esp32_brom_loads_and_executes() {
    let elf_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/esp32_brom.elf");
    assert!(
        elf_path.exists(),
        "BROM ELF fixture missing at {}",
        elf_path.display()
    );

    let mut bus = SystemBus::new();
    let cpu = configure_xtensa_esp32(&mut bus);
    let image = labwired_loader::load_elf(&elf_path).expect("parse BROM ELF");

    // Load BROM segments. For segments inside the ROM bank (RomThunkBank
    // — silently drops writes via `write()`), use the `preload_bytes`
    // bypass; for everything else, fall through to the regular bus write.
    let mut loaded_segments = 0;
    let mut skipped_segments: Vec<(u64, usize)> = Vec::new();
    for segment in &image.segments {
        let start = segment.start_addr as u32;
        let len = segment.data.len();
        let mut loaded_via_rom = false;
        for p in bus.peripherals.iter_mut() {
            if let Some(any) = p.dev.as_any_mut() {
                if let Some(rom) = any
                    .downcast_mut::<labwired_core::peripherals::esp32s3::rom_thunks::RomThunkBank>()
                {
                    let base = p.base as u32;
                    let size = p.size as u32;
                    if start >= base && (start as u64 + len as u64) <= (base as u64 + size as u64) {
                        rom.preload_bytes(start, &segment.data);
                        loaded_via_rom = true;
                        break;
                    }
                }
            }
        }
        if loaded_via_rom {
            loaded_segments += 1;
            eprintln!(
                "[BROM] loaded ROM segment @ 0x{:08x} ({} bytes)",
                start, len
            );
            continue;
        }
        // Try normal bus path for DRAM/IRAM/etc.
        let mut all_written = true;
        for (i, &byte) in segment.data.iter().enumerate() {
            if bus.write_u8(start as u64 + i as u64, byte).is_err() {
                all_written = false;
                break;
            }
        }
        if all_written {
            loaded_segments += 1;
            eprintln!(
                "[BROM] loaded RAM segment @ 0x{:08x} ({} bytes)",
                start, len
            );
        } else {
            skipped_segments.push((start as u64, len));
            eprintln!(
                "[BROM] SKIPPED segment @ 0x{:08x} ({} bytes) — region not mapped",
                start, len
            );
        }
    }
    assert!(loaded_segments > 0, "no BROM segments loaded");
    eprintln!(
        "[BROM] {} loaded, {} skipped",
        loaded_segments,
        skipped_segments.len()
    );

    let mut machine = Machine::new(cpu, bus);

    // Verify what's actually in the bus at the reset vector.
    let b0 = machine.bus.read_u8(0x4000_0400).unwrap();
    let b1 = machine.bus.read_u8(0x4000_0401).unwrap();
    let b2 = machine.bus.read_u8(0x4000_0402).unwrap();
    let b3 = machine.bus.read_u8(0x4000_0403).unwrap();
    eprintln!(
        "[BROM] bus bytes at reset vector 0x40000400: {:02x} {:02x} {:02x} {:02x}",
        b0, b1, b2, b3
    );

    // ESP32 rev3 BROM entry point — the _ResetVector at 0x40000400.
    machine.cpu.set_pc(0x4000_0400);
    machine.cpu.set_sp(0x3FFE_0000);

    // Step a few thousand instructions and report where we end up.
    // Any stall PC inside the BROM range (0x4000_0000..0x4007_0000) tells
    // us which BROM function the firmware reached before hitting an
    // unmodeled peripheral. A stall outside the BROM range means BROM
    // jumped to flash (highly unlikely in a few thousand steps) or
    // garbage (= probable peripheral-state bug).
    let start_pc = machine.cpu.get_pc();
    eprintln!("[BROM] starting execution at PC=0x{:08x}", start_pc);
    let mut step_err = None;
    for i in 0..50_000 {
        if let Err(e) = machine.step() {
            step_err = Some((i, e));
            break;
        }
    }
    let final_pc = machine.cpu.get_pc();
    match step_err {
        Some((cycle, err)) => {
            eprintln!(
                "[BROM] CPU exception at cycle {}, PC=0x{:08x}: {}",
                cycle, final_pc, err
            );
        }
        None => {
            eprintln!(
                "[BROM] 50000 steps completed without exception, final PC=0x{:08x}",
                final_pc
            );
        }
    }

    // The smoke test passes if we got past the loader; the diagnostic
    // output above is the actual Phase 1 deliverable.
}
