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

/// Re-parse the ELF directly with goblin to recover (vaddr, paddr) pairs
/// — `labwired_loader::load_elf` only surfaces `p_paddr` (LMA), which is
/// usually what flash programming wants. The ESP32 BROM has rodata
/// segments where VMA (0x3FF96000-range, data-bus view) differs from
/// LMA (0x40066000-range, instruction-bus view) because the ROM is
/// dual-mapped in hardware. Returning (vaddr, paddr) here lets the test
/// load each segment at both addresses so BROM code that reads its own
/// rodata via the data-bus alias finds the bytes.
fn brom_segment_addr_pairs(elf_bytes: &[u8]) -> Vec<(u64, u64)> {
    use goblin::elf::program_header::PT_LOAD;
    let elf = goblin::elf::Elf::parse(elf_bytes).expect("parse BROM ELF for VMA pairs");
    elf.program_headers
        .iter()
        .filter(|ph| ph.p_type == PT_LOAD && ph.p_filesz > 0)
        .map(|ph| (ph.p_vaddr, ph.p_paddr))
        .collect()
}

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
    let elf_bytes = std::fs::read(&elf_path).expect("read BROM ELF for vaddr lookup");
    let vaddr_for_paddr: std::collections::HashMap<u64, u64> = brom_segment_addr_pairs(&elf_bytes)
        .into_iter()
        .map(|(vaddr, paddr)| (paddr, vaddr))
        .collect();

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
                    .downcast_mut::<labwired_core::peripherals::esp_xtensa_common::rom_thunks::RomThunkBank>()
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

    // ESP32 dual-maps ROM at both an execution address (LMA, 0x4006xxxx)
    // and a data-bus alias (VMA, 0x3FF9xxxx) so code can both run from
    // and read constants out of the same physical bits. Our loader hands
    // us only the LMA. For segments where VMA != LMA, mirror the bytes
    // at the VMA too — otherwise things like the gpio_pad_unhold jump
    // table at 0x3FF9C174 read as zeros and the BROM `jx a8` lands at PC=0.
    let mut mirrored = 0;
    for segment in &image.segments {
        let paddr = segment.start_addr;
        let Some(&vaddr) = vaddr_for_paddr.get(&paddr) else {
            continue;
        };
        if vaddr == paddr || vaddr == 0 {
            continue;
        }
        let mut ok = true;
        for (i, &byte) in segment.data.iter().enumerate() {
            if bus.write_u8(vaddr + i as u64, byte).is_err() {
                ok = false;
                break;
            }
        }
        if ok {
            mirrored += 1;
            eprintln!(
                "[BROM] mirrored ROM rodata to VMA 0x{:08x} (LMA was 0x{:08x}, {} bytes)",
                vaddr,
                paddr,
                segment.data.len()
            );
        } else {
            eprintln!(
                "[BROM] FAILED to mirror segment LMA 0x{:08x} → VMA 0x{:08x}",
                paddr, vaddr
            );
        }
    }
    eprintln!("[BROM] mirrored {} VMA-aliased segments", mirrored);
    assert!(loaded_segments > 0, "no BROM segments loaded");
    eprintln!(
        "[BROM] {} loaded, {} skipped",
        loaded_segments,
        skipped_segments.len()
    );

    // (Skipped probe: patching `ets_unpack_flash_code_legacy_patch` at
    // 0x4000fb78 had no effect because the BROM never reaches that
    // function — the boot-mode-index bounds check in `main`
    // (~0x400078fd: `bgeu a4=15, a2=boot_index-1`) fails first when our
    // synthesized strap value maps to an out-of-range index, sending PC
    // straight to the "ets_main.c 404" trap. Fix belongs upstream of the
    // unpack call, not here. Tracked as labwired-core#2h-followup.)

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
    // Tolerate Xtensa general exceptions (ExceptionRaised): the simulator
    // already mutates PC → VECBASE+0x300 on exception entry, so the next
    // step() lands in the kernel exception vector. Real silicon delivers
    // ILL.N / unimplemented-opcode traps to the same vector, and the BROM
    // hands them off to `panic_helper`/`reset`. Stop only when the same PC
    // repeats too many times (handler is stuck spinning) or when we hit a
    // NotImplemented / bus error (genuine simulator gap).
    let mut step_err: Option<(usize, String)> = None;
    let mut last_distinct_trail: std::collections::VecDeque<u32> =
        std::collections::VecDeque::new();
    let mut last_pc = machine.cpu.get_pc();
    let mut same_pc_streak = 0usize;
    let mut visited_funcs: std::collections::BTreeMap<u32, usize> =
        std::collections::BTreeMap::new();
    // Last-N PC ring buffer so we can dump the trail right before any fault.
    let trail_len = 32usize;
    let mut pc_trail: std::collections::VecDeque<u32> = std::collections::VecDeque::new();
    let mut steps_executed = 0usize;
    for i in 0..1_000_000 {
        steps_executed = i + 1;
        let pc_now = machine.cpu.get_pc();
        pc_trail.push_back(pc_now);
        if pc_trail.len() > trail_len {
            pc_trail.pop_front();
        }
        // Bucket the program counter by 4 KiB to get a rough function map.
        let bucket = pc_now & !0xFFF;
        *visited_funcs.entry(bucket).or_insert(0) += 1;
        match machine.step() {
            Ok(()) => {
                let pc = machine.cpu.get_pc();
                if pc == last_pc {
                    same_pc_streak += 1;
                    if same_pc_streak > 64 {
                        eprintln!("[BROM] last 32 distinct PCs before stall at 0x{:08x}:", pc);
                        for p in last_distinct_trail.iter() {
                            eprintln!("  0x{:08x}", p);
                        }
                        step_err = Some((i, format!("PC stuck at 0x{:08x}", pc)));
                        break;
                    }
                } else {
                    same_pc_streak = 0;
                    last_pc = pc;
                    last_distinct_trail.push_back(pc);
                    if last_distinct_trail.len() > 32 {
                        last_distinct_trail.pop_front();
                    }
                }
            }
            Err(labwired_core::SimulationError::ExceptionRaised { cause, pc }) => {
                // Soft exception: log + continue. The CPU has already
                // re-pointed PC at the kernel exception vector.
                eprintln!(
                    "[BROM] EXCCAUSE={} raised at cycle {}, faulting PC=0x{:08x} (handler at 0x{:08x})",
                    cause,
                    i,
                    pc,
                    machine.cpu.get_pc()
                );
                eprintln!("[BROM] PC trail (last {} steps):", pc_trail.len());
                for p in pc_trail.iter() {
                    eprintln!("  PC=0x{:08x}", p);
                }
                last_pc = machine.cpu.get_pc();
                same_pc_streak = 0;
            }
            Err(e) => {
                step_err = Some((i, format!("{}", e)));
                break;
            }
        }
    }
    let final_pc = machine.cpu.get_pc();
    match step_err {
        Some((cycle, err)) => {
            eprintln!(
                "[BROM] simulator stalled at cycle {}, PC=0x{:08x}: {}",
                cycle, final_pc, err
            );
        }
        None => {
            eprintln!(
                "[BROM] 1000000 steps completed without fatal error, final PC=0x{:08x}",
                final_pc
            );
        }
    }
    eprintln!("[BROM] hot PC buckets (4 KiB) by cycles:");
    let mut buckets: Vec<(u32, usize)> = visited_funcs.into_iter().collect();
    buckets.sort_by_key(|b| std::cmp::Reverse(b.1));
    for (pc, count) in buckets.iter().take(10) {
        eprintln!("  0x{:08x}: {} cycles", pc, count);
    }

    eprintln!("[BROM] steps_executed={}", steps_executed);
    // Floor set at 50% of the observed 12 947-step run. If BROM regresses
    // (e.g. an unimplemented opcode now faults immediately) this fires fast.
    const STEP_FLOOR: usize = 6_400;
    assert!(
        steps_executed >= STEP_FLOOR,
        "BROM smoke ran only {} steps before stalling (floor {}); \
         a regression likely caused an early abort — check the step_err above",
        steps_executed,
        STEP_FLOOR
    );
}
