//! Diagnostic: dual-core Arduino-ESP32 L0 boot — where does APP_CPU go?
//!
//! ```text
//! cargo test -p labwired-core --test diag_esp32_dual_core_boot --release -- --ignored --nocapture
//! ```

use labwired_core::system::builder::build_esp32_system_from_manifest;
use labwired_core::{Bus, Cpu, Machine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[test]
#[ignore = "diagnostic only"]
fn diag_app_cpu_boot_progress() {
    // crates/core → labwired/core
    let core_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize core root");

    let elf = core_root.join("validation/arduino-matrix/out/esp32/L0_serial_boot/firmware.elf");
    let sys = core_root.join("validation/arduino-matrix/systems/esp32.yaml");
    assert!(
        elf.exists(),
        "missing {elf:?} — run arduino matrix compile for esp32 L0 first"
    );
    assert!(sys.exists(), "missing {sys:?}");

    let manifest = labwired_config::SystemManifest::from_file(&sys).expect("manifest");
    let (mut bus, pro, app) = build_esp32_system_from_manifest(&manifest, &sys).expect("build");
    let uart = Arc::new(Mutex::new(Vec::<u8>::new()));
    bus.attach_uart_tx_sink(uart.clone(), false);
    // Real partition table at flash 0x8000 + MMU seed for cache2phys (no OTA
    // firmware thunk). Matrix PIO emits partitions.bin next to the build.
    let partitions = core_root.join(
        "validation/arduino-matrix/out/_pio_work/esp32__L0_serial_boot/.pio/build/matrix/partitions.bin",
    );
    let pt_bytes = if partitions.exists() {
        Some(std::fs::read(&partitions).expect("read partitions.bin"))
    } else {
        eprintln!("[diag] WARNING: missing {partitions:?}");
        None
    };
    labwired_core::peripherals::esp32::flash_mmu::seed_esp32_flash_image(
        &mut bus,
        pt_bytes.as_deref(),
    )
    .expect("seed flash image");
    if let Some(ref b) = pt_bytes {
        eprintln!(
            "[diag] seeded partitions.bin ({} bytes) @ flash 0x8000 + app XIP MMU",
            b.len()
        );
    }
    let program = labwired_loader::load_elf(&elf).expect("elf");
    let mut machine = Machine::new(pro, bus).with_secondary_cpu(app);
    machine.load_firmware(&program).expect("load");
    machine.cpu.set_pc(program.entry_point as u32);
    machine.cpu.set_sp(0x3FFE_0000);
    if let Some(c1) = machine.cpu_secondary.as_mut() {
        c1.set_sp(0x3FFD_8000);
    }
    // Post-BROM DRAM seed (mirrors labwired-cli test path).
    let elf_bytes = std::fs::read(&elf).expect("read elf bytes");
    if let Some(addr) = labwired_loader::resolve_symbol_in_elf(&elf_bytes, "g_rom_flashchip") {
        use labwired_core::Bus;
        let base = addr as u64;
        let _ = machine.bus.write_u32(base, 0x0016_40EF);
        let _ = machine.bus.write_u32(base + 4, 4 * 1024 * 1024);
        let _ = machine.bus.write_u32(base + 8, 64 * 1024);
        let _ = machine.bus.write_u32(base + 12, 4 * 1024);
        let _ = machine.bus.write_u32(base + 16, 256);
        let _ = machine.bus.write_u32(base + 20, 0xFFFF);
        let id = machine.bus.read_u32(base).unwrap_or(0);
        let sz = machine.bus.read_u32(base + 4).unwrap_or(0);
        eprintln!("[diag] g_rom_flashchip @0x{addr:08x} id=0x{id:08x} size=0x{sz:x}");
    } else {
        eprintln!("[diag] WARNING: g_rom_flashchip symbol not found");
    }
    for name in ["g_ticks_per_us_pro", "g_ticks_per_us_app"] {
        if let Some(addr) = labwired_loader::resolve_symbol_in_elf(&elf_bytes, name) {
            use labwired_core::Bus;
            let _ = machine.bus.write_u32(addr as u64, 240);
        }
    }

    // CPU-model workaround (not a firmware flash-thunk): shadow-spill leaves
    // WindowStart bits set while physical ARs are clobbered; firmware
    // `xthal_window_spill_nw` then stores to a1-16 with a1==0 → 0xfffffff0.
    // Same install as e2e_ereader / e2e_wifi / CLI snapshot (FIDELITY Batch D).
    if let Some(pc) = labwired_loader::resolve_symbol_in_elf(&elf_bytes, "xthal_window_spill_nw") {
        machine
            .bus
            .install_flash_thunk(
                pc,
                labwired_core::peripherals::esp_xtensa_common::rom_thunks::xthal_window_spill_thunk,
            )
            .expect("install xthal_window_spill_nw");
        eprintln!("[diag] installed xthal_window_spill_nw CPU spill workaround @0x{pc:08x}");
    }
    // Halt on assert/abort so we see the real message instead of a cascade
    // (e.g. spinlock assert → corrupt path → 0x6b636f6c).
    for sym in [
        "panic_abort",
        "__assert_func",
        "abort",
        "__assert",
        "esp_system_abort",
    ] {
        if let Some(pc) = labwired_loader::resolve_symbol_in_elf(&elf_bytes, sym) {
            match machine.bus.install_flash_thunk(
                pc,
                labwired_core::peripherals::esp_xtensa_common::rom_thunks::abort_halt,
            ) {
                Ok(()) => eprintln!("[diag] installed abort_halt on {sym} @0x{pc:08x}"),
                Err(e) => eprintln!("[diag] FAIL install {sym} @0x{pc:08x}: {e}"),
            }
        } else {
            eprintln!("[diag] symbol {sym} not found");
        }
    }

    let mut last_pro = machine.cpu.get_pc();
    let mut last_app = machine
        .cpu_secondary
        .as_ref()
        .map(|c| (c.get_pc(), c.halted))
        .unwrap_or((0, true));
    let mut app_unhalted_at = None;

    // Past multi flash-IPC toward setup / LW_L0_OK.
    const N: u64 = 5_000_000;
    // Sketch / Arduino core markers (nm of this ELF).
    const PC_SETUP: u32 = 0x400d_15c0;
    const PC_APP_MAIN: u32 = 0x400d_3594;
    const PC_INIT_ARDUINO: u32 = 0x400d_1ee0;
    const PC_INIT_ARDUINO_RET: u32 = 0x400d_35ab; // app_main after initArduino call
    const PC_LOOP_TASK: u32 = 0x400d_354c;
    const PC_NVS_INIT: u32 = 0x400d_c3bc;
    const PC_NVS_INIT_RET: u32 = 0x400d_c3c7;
    const PC_CREATE_UNI: u32 = 0x400d_1e90;
    const PC_FLASH_BLOCK: u32 = 0x4008_1634;
    let mut seen_setup = false;
    let mut seen_app_main = false;
    let mut seen_init_arduino = false;
    let mut seen_init_arduino_ret = false;
    let mut seen_loop_task = false;
    let mut seen_nvs = false;
    let mut seen_nvs_ret = false;
    let mut seen_create = false;
    let mut flash_block_hits = 0u32;
    // After first free-list push, watch PageManager::load's [sp+24] (free-list
    // ptr). Corruption to a stack address is the 0xc0c00004 fault root cause.
    let mut watch_list_slot: Option<(u32, u32)> = None; // (addr, last_value)
    let mut ring: std::collections::VecDeque<(u64, u32, u32, bool)> =
        std::collections::VecDeque::with_capacity(32);
    for step in 1..=N {
        if let Err(e) = machine.step() {
            eprintln!("[diag] step {step} ERROR: {e}");
            eprintln!("[diag] last {} samples before error:", ring.len());
            for (s, p, a, h) in &ring {
                eprintln!("  step {s}: pro=0x{p:08x} app=0x{a:08x} halted={h}");
            }
            for i in 0..16u8 {
                let v = machine.cpu.regs.read_logical(i);
                eprintln!("[diag] pro a{i}=0x{v:08x}");
            }
            if let Some(app) = machine.cpu_secondary.as_ref() {
                for i in 0..16u8 {
                    let v = app.regs.read_logical(i);
                    eprintln!("[diag] app a{i}=0x{v:08x}");
                }
                eprintln!(
                    "[diag] app pc=0x{:08x} sp=0x{:08x} halted={}",
                    app.get_pc(),
                    app.regs.read_logical(1),
                    app.halted
                );
            }
            // Known DRAM handshake / flash-op flags from nm of this ELF.
            for (name, addr) in [
                ("s_resume_cores", 0x3ffc_2288u32),
                ("s_cpu_inited", 0x3ffc_2289),
                ("s_cpu_up", 0x3ffc_228b),
                ("s_system_full_inited", 0x3ffc_2384),
                ("s_flash_op_cpu", 0x3ffb_dc14),
                ("s_flash_op_can_start", 0x3ffc_220f),
                ("s_flash_op_complete", 0x3ffc_220e),
                ("port_xSchedulerRunning", 0x3ffc_27f0),
                // Heap / interrupt allocator state (nm of this ELF).
                ("vector_desc_head", 0x3ffc_24cc),
                ("registered_heaps", 0x3ffc_24b0),
                ("_heap_start", 0x3ffc_3198),
            ] {
                let b = machine.bus.read_u8(addr as u64).unwrap_or(0xEE);
                let w = machine.bus.read_u32(addr as u64).unwrap_or(0xEEEE_EEEE);
                eprintln!("[diag] {name}@0x{addr:08x}: byte=0x{b:02x} word=0x{w:08x}");
            }
            // Walk vector_desc_head list (max 16 nodes).
            let mut vd = machine.bus.read_u32(0x3ffc_24cc).unwrap_or(0);
            for i in 0..16 {
                if vd == 0 {
                    break;
                }
                let w0 = machine.bus.read_u32(vd as u64).unwrap_or(0xEEEE_EEEE);
                let shared = machine.bus.read_u32(vd as u64 + 4).unwrap_or(0xEEEE_EEEE);
                let next = machine.bus.read_u32(vd as u64 + 8).unwrap_or(0xEEEE_EEEE);
                eprintln!(
                    "[diag] vd[{i}] @0x{vd:08x}: flags/cpu/int=0x{w0:08x} shared=0x{shared:08x} next=0x{next:08x}"
                );
                if next == 0x6b63_6f6c || !(0x3ff0_0000..0x4000_0000).contains(&next) && next != 0 {
                    eprintln!("[diag]   *** corrupt next pointer");
                    break;
                }
                vd = next;
            }
            // Dump registered_heaps SLIST (heap_t: caps[3], start, end, heap*,
            // portMUX{owner,count}, sle_next) — 9×u32 = 36 bytes.
            let mut hp = machine.bus.read_u32(0x3ffc_24b0).unwrap_or(0);
            for i in 0..8 {
                if hp == 0 {
                    break;
                }
                let mut words = [0u32; 9];
                for (j, w) in words.iter_mut().enumerate() {
                    *w = machine
                        .bus
                        .read_u32(hp as u64 + (j as u64) * 4)
                        .unwrap_or(0xEEEE_EEEE);
                }
                let start = words[3];
                let end = words[4];
                let handle = words[5];
                let owner = words[6];
                let count = words[7];
                let next = words[8];
                eprintln!(
                    "[diag] heap[{i}] @0x{hp:08x}: start=0x{start:08x} end=0x{end:08x} \
                     size=0x{:x} handle=0x{handle:08x} lock.owner=0x{owner:08x} \
                     lock.count=0x{count:x} next=0x{next:08x}",
                    end.wrapping_sub(start)
                );
                if next == 0 || next == hp || !(0x3ff0_0000..0x4000_0000).contains(&next) {
                    if next != 0 && next != hp {
                        eprintln!("[diag]   *** corrupt heap next");
                    }
                    break;
                }
                hp = next;
            }
            // Dump vector_desc at head fully + first 32 bytes around APP a6
            let a6 = machine
                .cpu_secondary
                .as_ref()
                .map(|c| c.regs.read_logical(6))
                .unwrap_or(0);
            if a6 != 0 {
                for off in (0..32).step_by(16) {
                    let mut line = String::new();
                    for j in 0..4 {
                        let w = machine
                            .bus
                            .read_u32(a6 as u64 + off + j * 4)
                            .unwrap_or(0xEEEE_EEEE);
                        line.push_str(&format!(" {w:08x}"));
                    }
                    eprintln!("[diag] app_a6+{off:02x}:{line}");
                }
            }
            // Global spinlocks + PRIDs
            for (name, addr) in [
                ("spinlock@3ffbdc18", 0x3ffb_dc18u32),
                ("spinlock@3ffbdcb4", 0x3ffb_dcb4),
                ("xKernelLock", 0x3ffb_dd60),
                ("lock_init_spinlock", 0x3ffb_dd68),
            ] {
                let owner = machine.bus.read_u32(addr as u64).unwrap_or(0xEEEE_EEEE);
                let count = machine.bus.read_u32(addr as u64 + 4).unwrap_or(0xEEEE_EEEE);
                eprintln!("[diag] {name}: owner=0x{owner:08x} count=0x{count:x}");
            }
            let pro_prid = machine.cpu.sr.read(labwired_core::cpu::xtensa_sr::PRID);
            let app_prid = machine
                .cpu_secondary
                .as_ref()
                .map(|c| c.sr.read(labwired_core::cpu::xtensa_sr::PRID))
                .unwrap_or(0);
            eprintln!("[diag] PRID pro=0x{pro_prid:08x} app=0x{app_prid:08x}");
            // APP stack: bind does `l32i a5, a1, 128` for the first stack arg
            // (statusreg / ret_handle plumbing). Dump that slot + nearby.
            if let Some(app) = machine.cpu_secondary.as_ref() {
                let sp = app.regs.read_logical(1) as u64;
                for base in [sp, sp + 128, sp + 128 - 48] {
                    for off in (0..32).step_by(16) {
                        let mut line = String::new();
                        for j in 0..4 {
                            let w = machine
                                .bus
                                .read_u32(base + off + j * 4)
                                .unwrap_or(0xEEEE_EEEE);
                            line.push_str(&format!(" {w:08x}"));
                        }
                        eprintln!("[diag] app_stack@0x{:08x}+{off:02x}:{line}", base as u32);
                    }
                }
            }
            // Peek at _heap_start region (multi_heap control block).
            for off in (0..64).step_by(16) {
                let base = 0x3ffc_3198u64 + off;
                let mut line = String::new();
                for j in 0..4 {
                    let w = machine.bus.read_u32(base + j * 4).unwrap_or(0xEEEE_EEEE);
                    line.push_str(&format!(" {w:08x}"));
                }
                eprintln!("[diag] heap_mem+{off:02x}:{line}");
            }
            // Scan a-regs + stack for pointers into .flash.rodata (assert strings).
            let mut ptrs = Vec::new();
            for i in 0..16u8 {
                ptrs.push(machine.cpu.regs.read_logical(i));
            }
            let sp = machine.cpu.regs.read_logical(1); // a1 = SP
            for off in (0..256).step_by(4) {
                if let Ok(w) = machine.bus.read_u32((sp + off) as u64) {
                    ptrs.push(w);
                }
            }
            for (idx, v) in ptrs.iter().enumerate() {
                let v = *v;
                if (0x3f40_0000..0x3f42_0000).contains(&v) {
                    let mut buf = Vec::new();
                    for off in 0..160u32 {
                        match machine.bus.read_u8((v + off) as u64) {
                            Ok(b) if b != 0 && b < 0x7f => buf.push(b),
                            Ok(0) => break,
                            _ => break,
                        }
                    }
                    if let Ok(s) = std::str::from_utf8(&buf) {
                        if s.len() > 4 {
                            eprintln!("[diag] ptr#{idx} 0x{v:08x}: {s:?}");
                        }
                    }
                }
            }
            break;
        }
        let pro_pc = machine.cpu.get_pc();
        let (app_pc, app_halted) = machine
            .cpu_secondary
            .as_ref()
            .map(|c| (c.get_pc(), c.halted))
            .unwrap_or((0, true));

        // Watch a5 across CALL8 malloc / critical enter inside
        // esp_intr_alloc_intrstatus_bind (window-restore corruption check).
        if let Some(app) = machine.cpu_secondary.as_ref() {
            const WATCH: &[(u32, &str)] = &[
                (0x400d_7fea, "after l32i a5,a1,128"),
                (0x400d_806f, "at call8 heap_caps_malloc"),
                (0x4008_245c, "heap_caps_malloc entry"),
                (0x400d_8072, "after heap_caps_malloc ret"),
                (0x400d_807f, "after EnterCritical ret"),
                (0x400d_8085, "before beqz a5"),
                (0x400d_8087, "l32i a8,a5,0"),
                (0x400d_8094, "l32i a7,a8,0 FAULT-SITE"),
                // esp_cpu_compare_and_set ENTRY — a2 must be &lock, not 0/1
                (0x4008_5670, "compare_and_set ENTRY"),
                // just after ENTRY window rotate (next ins)
                (0x4008_5673, "compare_and_set post-ENTRY"),
                // xPortEnterCriticalTimeout entry — a10 is mux*
                (0x4008_81dc, "EnterCriticalTimeout ENTRY"),
                // ulTaskGenericNotifyTake: a6 must stay &xKernelLock (0x3ffbdd60)
                (0x4008_a236, "NotifyTake before 1st EnterCrit"),
                (0x4008_a239, "NotifyTake after 1st EnterCrit"),
                (0x4008_a268, "NotifyTake before AddDelayed"),
                (0x4008_a271, "NotifyTake before yield IPI"),
                (0x4008_a276, "NotifyTake before ExitCrit"),
                (0x4008_a279, "NotifyTake after ExitCrit ret"),
                (0x4008_a27e, "NotifyTake before 2nd EnterCrit"),
                // vPortExitCritical
                (0x4008_82e8, "ExitCrit ENTRY"),
                (0x4008_8336, "ExitCrit RETW nested"),
                (0x4008_834a, "ExitCrit Callx8 xtos"),
                (0x4008_834d, "ExitCrit after xtos"),
                (0x4008_8350, "ExitCrit RETW final"),
                // FreeRTOS SP save / block sites (TCB↔stack desync hunt)
                (0x4008_85d2, "vPortYield save SP"),
                (0x4008_8477, "int_enter save SP"),
                (0x4008_8d0c, "AddDelayed ENTRY"),
                (0x4008_8530, "_frxt_dispatch ENTRY"),
                // Flash IPC on APP
                (0x4008_1634, "flash_op_block_func ENTRY"),
                (0x4008_1846, "ipc_task NotifyTake"),
                (0x4008_1849, "ipc_task after NotifyTake"),
                (0x4008_186f, "ipc_task callx8 noblock func"),
                // After callx8 returns: clear ready + s_no_block_func[cpu]
                (0x4008_1872, "ipc_task after func RET (before clear)"),
                (0x4008_187f, "ipc_task s32i clear noblock_fn"),
                (0x4008_1881, "ipc_task after clear noblock_fn"),
                (0x4008_a2c7, "NotifyTake RETW"),
            ];
            // Rate-limit the hot CAS watches after first few hits.
            static mut CAS_HITS: u32 = 0;
            for &(wpc, label) in WATCH {
                if app_pc == wpc {
                    let a2_early = app.regs.read_logical(2);
                    let a10_early = app.regs.read_logical(10);
                    if label.starts_with("NotifyTake") {
                        let a6 = app.regs.read_logical(6);
                        let wb = app.regs.windowbase();
                        let ws = app.regs.windowstart();
                        // Physical a6 for this WB
                        let phys_a6 = app.regs.physical((wb as usize * 4 + 6) & 63);
                        if a6 != 0x3ffb_dd60 {
                            eprintln!(
                                "[diag] a6 CORRUPT {label} step={step} a6=0x{a6:08x} phys_a6=0x{phys_a6:08x} a10=0x{a10_early:08x} wb={wb} ws=0x{ws:04x}"
                            );
                            // dump physical words around a6's slot
                            let base = (wb as usize * 4) & 63;
                            eprintln!(
                                "[diag]   phys[{}..]: {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x}",
                                base,
                                app.regs.physical(base),
                                app.regs.physical(base + 1),
                                app.regs.physical(base + 2),
                                app.regs.physical(base + 3),
                                app.regs.physical(base + 4),
                                app.regs.physical(base + 5),
                                app.regs.physical(base + 6),
                                app.regs.physical(base + 7),
                            );
                        } else if step < 250_000 {
                            eprintln!(
                                "[diag] a6 OK {label} step={step} a6=0x{a6:08x} wb={wb} ws=0x{ws:04x}"
                            );
                        }
                    }
                    if label.contains("clear noblock") || label.contains("before clear") {
                        let r32 = |a: u32| machine.bus.read_u32(a as u64).unwrap_or(0);
                        let r8 = |a: u32| machine.bus.read_u8(a as u64).unwrap_or(0);
                        let a4v = app.regs.read_logical(4);
                        let a8v = app.regs.read_logical(8);
                        let a6v = app.regs.read_logical(6);
                        eprintln!(
                            "[diag] CLEAR {label} step={step} a4=0x{a4v:08x} a8=0x{a8v:08x} a6=0x{a6v:08x} \
                             *func1=0x{:08x} ready1={} wb={} ws=0x{:04x}",
                            r32(0x3ffc_2228),
                            r8(0x3ffc_2221),
                            app.regs.windowbase(),
                            app.regs.windowstart()
                        );
                    }
                    if label == "NotifyTake RETW" || label == "ipc_task after NotifyTake" {
                        let sp = app.regs.read_logical(1);
                        let r32 = |a: u32| machine.bus.read_u32(a as u64).unwrap_or(0);
                        eprintln!(
                            "[diag] UFCHK {label} step={step} sp=0x{sp:08x} a0=0x{:08x} a1=0x{sp:08x} \
                             a2=0x{:08x} a4=0x{:08x} a5=0x{:08x} a6=0x{:08x} wb={} ws=0x{:04x}",
                            app.regs.read_logical(0),
                            app.regs.read_logical(2),
                            app.regs.read_logical(4),
                            app.regs.read_logical(5),
                            app.regs.read_logical(6),
                            app.regs.windowbase(),
                            app.regs.windowstart()
                        );
                        if sp >= 48 {
                            eprintln!(
                                "[diag]   mem[sp-16..]= {:08x} {:08x} {:08x} {:08x}  mem[sp-32..]= {:08x} {:08x} {:08x} {:08x}",
                                r32(sp - 16),
                                r32(sp - 12),
                                r32(sp - 8),
                                r32(sp - 4),
                                r32(sp - 32),
                                r32(sp - 28),
                                r32(sp - 24),
                                r32(sp - 20),
                            );
                            let frame_a1 = r32(sp - 12); // a1 field of a0-a3 save
                            if frame_a1 >= 12 {
                                let parent = r32(frame_a1 - 12);
                                eprintln!(
                                    "[diag]   save.a1=0x{frame_a1:08x} parent=0x{parent:08x} parent-32=[{:08x} {:08x} {:08x} {:08x}]",
                                    r32(parent.wrapping_sub(32)),
                                    r32(parent.wrapping_sub(28)),
                                    r32(parent.wrapping_sub(24)),
                                    r32(parent.wrapping_sub(20)),
                                );
                            }
                        }
                    }
                    // TCB / SP desync: log current TCB, SP, stack base, list container.
                    if label.contains("save SP")
                        || label.starts_with("AddDelayed")
                        || label.starts_with("_frxt_dispatch")
                        || label.contains("before AddDelayed")
                    {
                        let r32 = |a: u32| machine.bus.read_u32(a as u64).unwrap_or(0);
                        let tcb1 = r32(0x3ffc_27cc);
                        let sp = app.regs.read_logical(1);
                        let top = if tcb1 != 0 { r32(tcb1) } else { 0 };
                        let stack = if tcb1 != 0 { r32(tcb1 + 0x30) } else { 0 };
                        // name at TCB+0x34 (approx for this ELF — abort_halt uses +0x34 for "IDLE")
                        let name_w = if tcb1 != 0 { r32(tcb1 + 0x34) } else { 0 };
                        let state_cont = if tcb1 != 0 { r32(tcb1 + 0x14) } else { 0 };
                        let idle1 = r32(0x3ffc_2524);
                        let ipc1 = r32(0x3ffc_2254); // s_ipc_task_handle may be array
                        let idle1_top = if idle1 != 0 { r32(idle1) } else { 0 };
                        let idle1_cont = if idle1 != 0 { r32(idle1 + 0x14) } else { 0 };
                        let shared = idle1 != 0 && tcb1 != idle1 && top == idle1_top && top != 0;
                        eprintln!(
                            "[diag] TCB {label} step={step} cur1=0x{tcb1:08x} sp=0x{sp:08x} top=0x{top:08x} \
                             stack=0x{stack:08x} cont=0x{state_cont:08x} namew=0x{name_w:08x} \
                             idle1=0x{idle1:08x} idle1_top=0x{idle1_top:08x} idle1_cont=0x{idle1_cont:08x} \
                             ipc_h=0x{ipc1:08x} shared_top={shared}"
                        );
                    }
                    if label.starts_with("compare_and_set") || label.starts_with("EnterCritical") {
                        let hits = unsafe {
                            CAS_HITS += 1;
                            CAS_HITS
                        };
                        // For CAS ENTRY, arg is a10 (pre-ENTRY). For post-ENTRY, a2.
                        let arg = if label.contains("post-ENTRY") {
                            a2_early
                        } else if label.starts_with("EnterCritical") {
                            a10_early
                        } else {
                            a10_early // CAS pre-ENTRY: arg in a10
                        };
                        let corrupt = !(0x3ff0_0000..0x4000_0000).contains(&arg) && arg != 0;
                        if !corrupt && hits > 12 && hits % 100_000 != 0 {
                            continue;
                        }
                        if corrupt {
                            let a0 = app.regs.read_logical(0);
                            let a8 = app.regs.read_logical(8);
                            let ret = (a8 & 0x3fff_ffff) | 0x4000_0000;
                            eprintln!(
                                "[diag] CAS CORRUPT {label} step={step} arg=0x{arg:08x} a2=0x{a2_early:08x} a10=0x{a10_early:08x} a0=0x{a0:08x} a8=0x{a8:08x} ret≈0x{ret:08x} hits={hits}"
                            );
                        }
                    }
                    let a2 = a2_early;
                    let a3 = app.regs.read_logical(3);
                    let a4 = app.regs.read_logical(4);
                    let a5 = app.regs.read_logical(5);
                    let a6 = app.regs.read_logical(6);
                    let a10 = app.regs.read_logical(10);
                    let a11 = app.regs.read_logical(11);
                    let a12 = app.regs.read_logical(12);
                    let wb = app.regs.windowbase();
                    let ws = app.regs.windowstart();
                    let callinc = app.ps.callinc();
                    let d12 = app.regs.shadow_depth(12);
                    let d13 = app.regs.shadow_depth(13);
                    let p = |i: usize| app.regs.physical(i);
                    eprintln!(
                        "[diag] WATCH {label} step={step} a2=0x{a2:08x} a3=0x{a3:08x} a4=0x{a4:08x} a5=0x{a5:08x} a6=0x{a6:08x} a10=0x{a10:08x} a11=0x{a11:08x} a12=0x{a12:08x} wb={wb} ws=0x{ws:04x} callinc={callinc} sh12={d12} sh13={d13}"
                    );
                    if (0x3ff0_0000..0x4000_0000).contains(&a2) {
                        let o = machine.bus.read_u32(a2 as u64).unwrap_or(0);
                        let c = machine.bus.read_u32(a2 as u64 + 4).unwrap_or(0);
                        eprintln!("[diag]   *a2 owner=0x{o:08x} count=0x{c:x}");
                    }
                    if label.contains("a5") || label.contains("malloc") || label.contains("beqz") {
                        eprintln!(
                            "[diag]   phys52-55(a4-7@wb12)=[{:08x},{:08x},{:08x},{:08x}] shadow13_top={}",
                            p(52),
                            p(53),
                            p(54),
                            p(55),
                            app.regs
                                .shadow_stacks()[13]
                                .last()
                                .map(|s| format!("{:08x?}", s))
                                .unwrap_or_else(|| "none".into())
                        );
                    }
                }
            }
        }

        if app_unhalted_at.is_none() && !app_halted {
            app_unhalted_at = Some(step);
            eprintln!(
                "[diag] APP unhalted at step {step}: pro_pc=0x{pro_pc:08x} app_pc=0x{app_pc:08x}"
            );
        }

        let changed = pro_pc != last_pro || app_pc != last_app.0 || app_halted != last_app.1;
        if changed {
            if ring.len() == 32 {
                ring.pop_front();
            }
            ring.push_back((step, pro_pc, app_pc, app_halted));
        }
        if changed && (step < 80_000 || step % 100_000 == 0) {
            eprintln!(
                "[diag] step {step}: pro=0x{pro_pc:08x} app=0x{app_pc:08x} app_halted={app_halted}"
            );
        }
        if !seen_app_main && pro_pc == PC_APP_MAIN {
            seen_app_main = true;
            eprintln!("[diag] HIT app_main @ step {step}");
        }
        if !seen_init_arduino && pro_pc == PC_INIT_ARDUINO {
            seen_init_arduino = true;
            eprintln!("[diag] HIT initArduino @ step {step}");
        }
        if !seen_init_arduino_ret && pro_pc == PC_INIT_ARDUINO_RET {
            seen_init_arduino_ret = true;
            eprintln!("[diag] HIT initArduino RETURNED @ step {step}");
        }
        if !seen_nvs && pro_pc == PC_NVS_INIT {
            seen_nvs = true;
            eprintln!("[diag] HIT nvs_flash_init @ step {step}");
        }
        // nvs::intrusive_list<Page>::push_back — diagnose free-list corruption.
        // Sample at ENTRY PC: CALL8 args still live in a10/a11 (ENTRY not yet run).
        const PC_PAGE_PUSH: u32 = 0x400e_dd74;
        if pro_pc == PC_PAGE_PUSH {
            let list = machine.cpu.regs.read_logical(10);
            let node = machine.cpu.regs.read_logical(11);
            let head = machine.bus.read_u32(list as u64).unwrap_or(0xEEEE_EEEE);
            let tail = machine.bus.read_u32(list as u64 + 4).unwrap_or(0xEEEE_EEEE);
            let size = machine.bus.read_u32(list as u64 + 8).unwrap_or(0xEEEE_EEEE);
            let p0 = machine.bus.read_u32(node as u64).unwrap_or(0xEEEE_EEEE);
            let p4 = machine.bus.read_u32(node as u64 + 4).unwrap_or(0xEEEE_EEEE);
            let p12 = machine
                .bus
                .read_u32(node as u64 + 12)
                .unwrap_or(0xEEEE_EEEE);
            let sp = machine.cpu.regs.read_logical(1);
            let saved_list = machine.bus.read_u32(sp as u64 + 24).unwrap_or(0xEEEE_EEEE);
            eprintln!(
                "[diag] Page::list push_back step={step} a10/list=0x{list:08x} a11/node=0x{node:08x} \
                 head=0x{head:08x} tail=0x{tail:08x} size={size} \
                 node[0]=0x{p0:08x} node[4]=0x{p4:08x} state=0x{p12:08x} \
                 sp=0x{sp:08x} [sp+24]=0x{saved_list:08x}"
            );
            // Arm watch on load's free-list slot (sp+24 of PageManager::load).
            if watch_list_slot.is_none() && list > 0x3ff8_0000 && list < 0x4000_0000 {
                let slot = sp.wrapping_add(24);
                watch_list_slot = Some((slot, saved_list));
                eprintln!(
                    "[diag] WATCH free-list slot @0x{slot:08x} = 0x{saved_list:08x} (expect heap list)"
                );
            }
        }
        // Detect free-list pointer corruption as soon as it happens.
        if let Some((slot, prev)) = watch_list_slot {
            let cur = machine.bus.read_u32(slot as u64).unwrap_or(0xEEEE_EEEE);
            if cur != prev {
                let psp = machine.cpu.regs.read_logical(1);
                let app_sp = machine
                    .cpu_secondary
                    .as_ref()
                    .map(|a| a.regs.read_logical(1))
                    .unwrap_or(0);
                eprintln!(
                    "[diag] FREE-LIST SLOT CLOBBER step={step} @0x{slot:08x}: \
                     0x{prev:08x} -> 0x{cur:08x} pro_pc=0x{pro_pc:08x} app_pc=0x{app_pc:08x} \
                     pro_sp=0x{psp:08x} app_sp=0x{app_sp:08x}"
                );
                eprintln!("[diag]   ring (last steps before clobber):");
                for (s, p, a, h) in ring.iter().rev().take(12) {
                    eprintln!("[diag]     step {s}: pro=0x{p:08x} app=0x{a:08x} halted={h}");
                }
                for i in 0..16u8 {
                    let v = machine.cpu.regs.read_logical(i);
                    eprintln!("[diag]   pro a{i}=0x{v:08x}");
                }
                // Dump 64B around the slot.
                for off in (0u32..64).step_by(16) {
                    let base = slot.wrapping_sub(16).wrapping_add(off);
                    let w0 = machine.bus.read_u32(base as u64).unwrap_or(0);
                    let w1 = machine.bus.read_u32(base as u64 + 4).unwrap_or(0);
                    let w2 = machine.bus.read_u32(base as u64 + 8).unwrap_or(0);
                    let w3 = machine.bus.read_u32(base as u64 + 12).unwrap_or(0);
                    eprintln!("[diag]   mem[0x{base:08x}]: {w0:08x} {w1:08x} {w2:08x} {w3:08x}");
                }
                watch_list_slot = Some((slot, cur));
            }
        }
        if !seen_nvs_ret && pro_pc == PC_NVS_INIT_RET {
            seen_nvs_ret = true;
            let a2 = machine.cpu.regs.read_logical(2);
            eprintln!("[diag] HIT nvs_flash_init RET a2/retval=0x{a2:08x} @ step {step}");
        }
        if !seen_create && pro_pc == PC_CREATE_UNI {
            seen_create = true;
            eprintln!("[diag] HIT xTaskCreateUniversal @ step {step}");
        }
        if pro_pc == PC_FLASH_BLOCK || app_pc == PC_FLASH_BLOCK {
            flash_block_hits += 1;
            if flash_block_hits <= 5 || flash_block_hits % 20 == 0 {
                eprintln!(
                    "[diag] HIT flash_op_block_func #{flash_block_hits} step={step} pro=0x{pro_pc:08x} app=0x{app_pc:08x}"
                );
            }
        }
        if !seen_loop_task && (pro_pc == PC_LOOP_TASK || app_pc == PC_LOOP_TASK) {
            seen_loop_task = true;
            eprintln!("[diag] HIT loopTask @ step {step} pro=0x{pro_pc:08x} app=0x{app_pc:08x}");
        }
        if !seen_setup && (pro_pc == PC_SETUP || app_pc == PC_SETUP) {
            seen_setup = true;
            eprintln!("[diag] HIT setup() @ step {step} pro=0x{pro_pc:08x} app=0x{app_pc:08x}");
        }
        // Serial.println path
        const PC_UART_WRITE_BUF: u32 = 0x400d_2ce0;
        const PC_UART_WRITE_BYTES: u32 = 0x400d_abf0;
        const PC_UART_HAL_WRITE_TXFIFO: u32 = 0x400d_73cc;
        const PC_HS_BEGIN: u32 = 0x400d_1838;
        if pro_pc == PC_HS_BEGIN || app_pc == PC_HS_BEGIN {
            eprintln!("[diag] HIT HardwareSerial::begin @ step {step} pro=0x{pro_pc:08x} app=0x{app_pc:08x}");
        }
        if pro_pc == PC_UART_WRITE_BUF || app_pc == PC_UART_WRITE_BUF {
            eprintln!(
                "[diag] HIT uartWriteBuf @ step {step} pro=0x{pro_pc:08x} app=0x{app_pc:08x}"
            );
            // Snapshot UART0 + DPORT map for source 34 (ETS_UART0_INTR_SOURCE).
            let u0 = 0x3FF4_0000u64;
            let fifo_st = machine.bus.read_u32(u0 + 0x1c).unwrap_or(0);
            let int_raw = machine.bus.read_u32(u0 + 0x04).unwrap_or(0);
            let int_st = machine.bus.read_u32(u0 + 0x08).unwrap_or(0);
            let int_ena = machine.bus.read_u32(u0 + 0x0c).unwrap_or(0);
            let clkdiv = machine.bus.read_u32(u0 + 0x14).unwrap_or(0);
            let p_uart0 = machine.bus.read_u32(0x3ffc_2928).unwrap_or(0);
            let serial0_uart = machine.bus.read_u32(0x3ffc_1dc0 + 20).unwrap_or(0);
            let dport = 0x3FF0_0000u64;
            let pro_map34 = machine.bus.read_u32(dport + 0x104 + 34 * 4).unwrap_or(0);
            let app_map34 = machine.bus.read_u32(dport + 0x208 + 34 * 4).unwrap_or(0);
            let pend0 = machine.bus.pending_cpu_irqs(0);
            let pend1 = machine.bus.pending_cpu_irqs(1);
            eprintln!(
                "[diag]   UART0 status=0x{fifo_st:08x} int_raw=0x{int_raw:08x} int_st=0x{int_st:08x} \
                 int_ena=0x{int_ena:08x} clkdiv=0x{clkdiv:08x} p_uart_obj[0]=0x{p_uart0:08x} \
                 Serial0._uart=0x{serial0_uart:08x}"
            );
            eprintln!(
                "[diag]   DPORT UART0 map pro=0x{pro_map34:x} app=0x{app_map34:x} \
                 pending_cpu_irqs pro=0x{pend0:08x} app=0x{pend1:08x}"
            );
        }
        if pro_pc == PC_UART_WRITE_BYTES || app_pc == PC_UART_WRITE_BYTES {
            let a2 = machine
                .cpu_secondary
                .as_ref()
                .map(|c| c.regs.read_logical(2))
                .unwrap_or(0);
            let a3 = machine
                .cpu_secondary
                .as_ref()
                .map(|c| c.regs.read_logical(3))
                .unwrap_or(0);
            let a4 = machine
                .cpu_secondary
                .as_ref()
                .map(|c| c.regs.read_logical(4))
                .unwrap_or(0);
            let p_uart0 = machine.bus.read_u32(0x3ffc_2928).unwrap_or(0);
            eprintln!(
                "[diag] HIT uart_write_bytes @ step {step} num=0x{a2:08x} src=0x{a3:08x} \
                 len=0x{a4:08x} p_uart_obj[0]=0x{p_uart0:08x}"
            );
        }
        if pro_pc == PC_UART_HAL_WRITE_TXFIFO || app_pc == PC_UART_HAL_WRITE_TXFIFO {
            let uart_st = machine.bus.read_u32(0x3ff4_001c).unwrap_or(0);
            eprintln!(
                "[diag] HIT uart_hal_write_txfifo @ step {step} UART0.STATUS=0x{uart_st:08x}"
            );
        }
        if step % 1_000_000 == 0 {
            let u = uart.lock().unwrap();
            let us = String::from_utf8_lossy(&u);
            eprintln!(
                "[diag] heartbeat step {step}: pro=0x{pro_pc:08x} app=0x{app_pc:08x} \
                 app_halted={app_halted} uart_len={} uart={us:?}",
                u.len()
            );
        }
        // Early success: sketch marker printed.
        {
            let u = uart.lock().unwrap();
            if u.windows(8).any(|w| w == b"LW_L0_OK") {
                eprintln!("[diag] SUCCESS LW_L0_OK at step {step}");
                break;
            }
        }
        // PRO flash-IPC watches — log every entry of the hot path (not only first).
        {
            static mut IPC_CALL_N: u32 = 0;
            static mut IPC_RET_N: u32 = 0;
            static mut WAIT_CS_N: u32 = 0;
            static mut NOTIFY_N: u32 = 0;
            static mut YIELD_N: u32 = 0;
            static mut LAST_WAIT_LOG: u64 = 0;
            let r32 = |a: u32| machine.bus.read_u32(a as u64).unwrap_or(0);
            let r8 = |a: u32| machine.bus.read_u8(a as u64).unwrap_or(0);
            let dump_ipc = |step: u64, label: &str, pro_a10: u32, app_pc: u32| {
                let ipc1 = r32(0x3ffc_2258); // s_ipc_task_handle[1]
                let idle1 = r32(0x3ffc_2524);
                let tcb1 = r32(0x3ffc_27cc);
                let ipc1_cont = if ipc1 != 0 { r32(ipc1 + 0x14) } else { 0 };
                let ipc1_top = if ipc1 != 0 { r32(ipc1) } else { 0 };
                let ipc1_prio = if ipc1 != 0 { r32(ipc1 + 0x2c) } else { 0 }; // uxPriority
                let ipc1_notif = if ipc1 != 0 {
                    r8(ipc1 + 0x100 + 92) // ucNotifyState approx — may be off
                } else {
                    0
                };
                let ready_flag = r8(0x3ffc_2221);
                let noblock_fn = r32(0x3ffc_2228);
                let from_cpu0 = r32(0x3ff0_00dc);
                let from_cpu1 = r32(0x3ff0_00e0);
                let can_start = r8(0x3ffc_220f);
                let complete = r8(0x3ffc_220e);
                let flash_cpu = r32(0x3ffb_dc14);
                let sus0 = r32(0x3ffc_2518);
                let sus1 = r32(0x3ffc_251c);
                let top_prio = r32(0x3ffc_2544);
                let pending_n = r32(0x3ffc_257c); // xPendingReadyList.uxNumberOfItems
                eprintln!(
                    "[diag] FLASH {label} step={step} pro_a10=0x{pro_a10:08x} app_pc=0x{app_pc:08x} \
                     can_start={can_start} complete={complete} flash_cpu={flash_cpu} \
                     ready1={ready_flag} noblock_fn=0x{noblock_fn:08x} \
                     from_cpu0=0x{from_cpu0:08x} from_cpu1=0x{from_cpu1:08x} \
                     ipc1=0x{ipc1:08x} ipc1_cont=0x{ipc1_cont:08x} ipc1_top=0x{ipc1_top:08x} \
                     ipc1_prio=0x{ipc1_prio:x} notif={ipc1_notif} \
                     cur1=0x{tcb1:08x} idle1=0x{idle1:08x} \
                     sus=[{sus0},{sus1}] top_prio={top_prio} pending_n={pending_n}"
                );
            };
            if pro_pc == 0x4008_16f2 {
                let n = unsafe {
                    IPC_CALL_N += 1;
                    IPC_CALL_N
                };
                if n <= 8 || n % 64 == 0 {
                    dump_ipc(
                        step,
                        &format!("PRO esp_ipc_call_nonblocking#{n}"),
                        machine.cpu.regs.read_logical(10),
                        app_pc,
                    );
                }
            }
            if pro_pc == 0x4008_16f5 {
                let n = unsafe {
                    IPC_RET_N += 1;
                    IPC_RET_N
                };
                if n <= 8 || n % 64 == 0 {
                    dump_ipc(
                        step,
                        &format!("PRO after ipc_call ret#{n}"),
                        machine.cpu.regs.read_logical(10),
                        app_pc,
                    );
                }
            }
            if pro_pc == 0x4008_16fd || pro_pc == 0x4008_1703 {
                // Tight can_start wait — rate-limit to every 50k steps.
                let last = unsafe { &mut LAST_WAIT_LOG };
                if step.saturating_sub(*last) >= 50_000 {
                    *last = step;
                    let n = unsafe {
                        WAIT_CS_N += 1;
                        WAIT_CS_N
                    };
                    dump_ipc(
                        step,
                        &format!("PRO wait can_start#{n}"),
                        machine.cpu.regs.read_logical(10),
                        app_pc,
                    );
                }
            }
            if pro_pc == 0x4008_a2cc {
                let n = unsafe {
                    NOTIFY_N += 1;
                    NOTIFY_N
                };
                if n <= 12 || n % 32 == 0 {
                    dump_ipc(
                        step,
                        &format!("PRO xTaskGenericNotify#{n}"),
                        machine.cpu.regs.read_logical(10),
                        app_pc,
                    );
                }
            }
            if pro_pc == 0x4008_21e4 || pro_pc == 0x4008_21dd {
                let n = unsafe {
                    YIELD_N += 1;
                    YIELD_N
                };
                if n <= 8 || n % 32 == 0 {
                    dump_ipc(
                        step,
                        &format!("PRO crosscore/FROM_CPU#{n}"),
                        machine.cpu.regs.read_logical(10),
                        app_pc,
                    );
                }
            }
            // Periodic snapshot after first post-sched flash window.
            if step >= 230_000 && step % 200_000 == 0 {
                dump_ipc(step, "PERIODIC", 0, app_pc);
            }
            // Track s_no_block_func[1] transitions (first flash IPC lifetime).
            static mut PREV_FN1: u32 = 0;
            static mut SAW_NONEMPTY: bool = false;
            static mut SAW_POST_CLEAR: bool = false;
            static mut LOGGED_REDIRTY: bool = false;
            let fn1 = r32(0x3ffc_2228);
            unsafe {
                if fn1 != PREV_FN1 && step >= 238_000 {
                    eprintln!(
                        "[diag] FN1_CHG step={step} 0x{PREV_FN1:08x}->0x{fn1:08x} \
                         pro=0x{pro_pc:08x} app=0x{app_pc:08x} \
                         pro_a2=0x{:08x} pro_a3=0x{:08x} pro_a4=0x{:08x} \
                         app_a4=0x{:08x} app_a8=0x{:08x}",
                        machine.cpu.regs.read_logical(2),
                        machine.cpu.regs.read_logical(3),
                        machine.cpu.regs.read_logical(4),
                        machine
                            .cpu_secondary
                            .as_ref()
                            .map(|c| c.regs.read_logical(4))
                            .unwrap_or(0),
                        machine
                            .cpu_secondary
                            .as_ref()
                            .map(|c| c.regs.read_logical(8))
                            .unwrap_or(0),
                    );
                    PREV_FN1 = fn1;
                }
                if fn1 != 0 {
                    SAW_NONEMPTY = true;
                }
                if SAW_NONEMPTY && fn1 == 0 {
                    SAW_POST_CLEAR = true;
                }
                if SAW_POST_CLEAR && fn1 != 0 && !LOGGED_REDIRTY {
                    LOGGED_REDIRTY = true;
                    eprintln!(
                        "[diag] REDIRTY s_no_block_func[1]=0x{fn1:08x} at step={step} \
                         pro=0x{pro_pc:08x} app=0x{app_pc:08x}"
                    );
                }
            }
        }
        last_pro = pro_pc;
        last_app = (app_pc, app_halted);
    }

    let pro_pc = machine.cpu.get_pc();
    let (app_pc, app_halted) = machine
        .cpu_secondary
        .as_ref()
        .map(|c| (c.get_pc(), c.halted))
        .unwrap_or((0, true));
    eprintln!("[diag] FINAL after steps (cap {N}):");
    eprintln!("  pro_pc=0x{pro_pc:08x}");
    eprintln!("  app_pc=0x{app_pc:08x} halted={app_halted}");
    eprintln!("  app_unhalted_at={app_unhalted_at:?}");
    eprintln!(
        "  hits: app_main={seen_app_main} initArduino={seen_init_arduino} \
         initArduino_ret={seen_init_arduino_ret} nvs={seen_nvs} nvs_ret={seen_nvs_ret} \
         createUni={seen_create} loopTask={seen_loop_task} setup={seen_setup} \
         flash_block_hits={flash_block_hits}"
    );
    // IPC / flash handshake residual state
    for (name, addr) in [
        ("s_no_block_func[0]", 0x3ffc_2224u32),
        ("s_no_block_func[1]", 0x3ffc_2228),
        ("s_no_block_ready[0]", 0x3ffc_2220),
        ("s_no_block_ready[1]", 0x3ffc_2221),
        ("loopTaskHandle", 0x3ffc_1e74),
        ("s_ipc_task_handle[1]", 0x3ffc_2258),
    ] {
        let w = machine.bus.read_u32(addr as u64).unwrap_or(0xEEEE_EEEE);
        let b = machine.bus.read_u8(addr as u64).unwrap_or(0xEE);
        eprintln!("  {name}@0x{addr:08x}: byte=0x{b:02x} word=0x{w:08x}");
    }
    {
        let u = uart.lock().unwrap();
        let us = String::from_utf8_lossy(&u);
        eprintln!("  uart_len={} uart={us:?}", u.len());
    }

    // Always dump handshake / scheduler state (not only on error).
    for (name, addr) in [
        ("s_resume_cores", 0x3ffc_2288u32),
        ("s_cpu_inited", 0x3ffc_2289),
        ("s_cpu_up", 0x3ffc_228b),
        ("s_system_full_inited", 0x3ffc_2384),
        ("s_flash_op_cpu", 0x3ffb_dc14),
        ("s_flash_op_can_start", 0x3ffc_220f),
        ("s_flash_op_complete", 0x3ffc_220e),
        ("s_flash_op_mutex", 0x3ffc_2210),
        ("s_other_cpu_startup_done", 0x3ffc_2508),
        ("port_xSchedulerRunning", 0x3ffc_27f0),
        ("esp_ipc_isr_start_fl", 0x3ffc_22b4),
        ("esp_ipc_isr_end_fl", 0x3ffb_dc20),
        ("esp_ipc_isr_finish_cmd", 0x3ffc_2298),
        ("s_no_block_func0", 0x3ffc_2224),
        ("s_no_block_func1", 0x3ffc_2228),
        ("s_ready0", 0x3ffc_2220),
        ("s_ready1", 0x3ffc_2221),
        ("uxSchedulerSuspended0", 0x3ffc_2518),
        ("uxSchedulerSuspended1", 0x3ffc_251c),
        ("uxTopReadyPriority", 0x3ffc_2544),
        ("xPendingReadyList_n", 0x3ffc_257c),
        ("xSuspendedTaskList_n", 0x3ffc_2550),
        ("FROM_CPU_0", 0x3ff0_00dc),
        ("FROM_CPU_1", 0x3ff0_00e0),
        ("s_ipc_task_handle0", 0x3ffc_2254),
        ("s_ipc_task_handle1", 0x3ffc_2258),
        ("pxCurrentTCB0", 0x3ffc_27c8),
        ("pxCurrentTCB1", 0x3ffc_27cc),
    ] {
        let b = machine.bus.read_u8(addr as u64).unwrap_or(0xEE);
        let w = machine.bus.read_u32(addr as u64).unwrap_or(0xEEEE_EEEE);
        eprintln!("[diag] {name}@0x{addr:08x}: byte=0x{b:02x} word=0x{w:08x}");
    }
    // ipc1 TCB detail
    {
        let ipc1 = machine.bus.read_u32(0x3ffc_2258).unwrap_or(0);
        if ipc1 != 0 {
            let top = machine.bus.read_u32(ipc1 as u64).unwrap_or(0);
            let cont = machine.bus.read_u32(ipc1 as u64 + 0x14).unwrap_or(0);
            let prio = machine.bus.read_u32(ipc1 as u64 + 0x2c).unwrap_or(0);
            let stack = machine.bus.read_u32(ipc1 as u64 + 0x30).unwrap_or(0);
            let name = machine.bus.read_u32(ipc1 as u64 + 0x34).unwrap_or(0);
            let core = machine.bus.read_u32(ipc1 as u64 + 0x38).unwrap_or(0);
            eprintln!(
                "[diag] ipc1_tcb@0x{ipc1:08x}: top=0x{top:08x} cont=0x{cont:08x} prio=0x{prio:x} \
                 stack=0x{stack:08x} name=0x{name:08x} coreish=0x{core:08x}"
            );
            // List end markers for classification
            for (ln, la) in [
                ("Suspended", 0x3ffc_2550u32),
                ("PendingReady", 0x3ffc_257c),
                ("Delayed1", 0x3ffc_25c0),
                ("Delayed2", 0x3ffc_25ac),
                ("Ready0", 0x3ffc_25d4),
            ] {
                let n = machine.bus.read_u32(la as u64).unwrap_or(0);
                eprintln!("[diag] list {ln}@0x{la:08x}: uxNumberOfItems={n}");
            }
        }
    }
    for i in 0..16u8 {
        let v = machine.cpu.regs.read_logical(i);
        eprintln!("[diag] pro a{i}=0x{v:08x}");
    }
    if let Some(app) = machine.cpu_secondary.as_ref() {
        for i in 0..16u8 {
            let v = app.regs.read_logical(i);
            eprintln!("[diag] app a{i}=0x{v:08x}");
        }
        let ps = app.ps.as_raw();
        let intenable = app.sr.read(labwired_core::cpu::xtensa_sr::INTENABLE);
        let interrupt = app.sr.read(labwired_core::cpu::xtensa_sr::INTERRUPT);
        eprintln!(
            "[diag] app ps=0x{ps:08x} intenable=0x{intenable:08x} interrupt=0x{interrupt:08x} wb={} ws=0x{:04x}",
            app.regs.windowbase(),
            app.regs.windowstart()
        );
    }
    {
        let ps = machine.cpu.ps.as_raw();
        let intenable = machine
            .cpu
            .sr
            .read(labwired_core::cpu::xtensa_sr::INTENABLE);
        let interrupt = machine
            .cpu
            .sr
            .read(labwired_core::cpu::xtensa_sr::INTERRUPT);
        eprintln!(
            "[diag] pro ps=0x{ps:08x} intenable=0x{intenable:08x} interrupt=0x{interrupt:08x} wb={} ws=0x{:04x}",
            machine.cpu.regs.windowbase(),
            machine.cpu.regs.windowstart()
        );
    }
    // APP CAS address: a2/a10 often hold the lock pointer for compare_and_set
    if let Some(app) = machine.cpu_secondary.as_ref() {
        for (label, r) in [("a2", 2u8), ("a3", 3), ("a10", 10), ("a11", 11)] {
            let p = app.regs.read_logical(r);
            if (0x3ff0_0000..0x4000_0000).contains(&p) {
                let w0 = machine.bus.read_u32(p as u64).unwrap_or(0);
                let w1 = machine.bus.read_u32(p as u64 + 4).unwrap_or(0);
                eprintln!("[diag] app {label}=0x{p:08x} -> [0x{w0:08x}, 0x{w1:08x}]");
            }
        }
        let sp = app.regs.read_logical(1) as u64;
        for off in (0..64).step_by(16) {
            let mut line = String::new();
            for j in 0..4 {
                let w = machine
                    .bus
                    .read_u32(sp + off + j * 4)
                    .unwrap_or(0xEEEE_EEEE);
                line.push_str(&format!(" {w:08x}"));
            }
            eprintln!("[diag] app_sp+{off:02x}:{line}");
        }
        // Physical AR file — recover pre-window arg pointers for CAS.
        for slot in 0..16u8 {
            let base = (slot as usize) * 4;
            let r = [
                app.regs.physical(base),
                app.regs.physical(base + 1),
                app.regs.physical(base + 2),
                app.regs.physical(base + 3),
            ];
            if r.iter().any(|v| (0x3ff0_0000..0x4000_0000).contains(v)) {
                eprintln!(
                    "[diag] app phys slot{slot}: {:08x} {:08x} {:08x} {:08x}",
                    r[0], r[1], r[2], r[3]
                );
            }
        }
    }
    for (name, addr) in [
        ("xKernelLock", 0x3ffb_dd60u32),
        ("lock_init_spinlock", 0x3ffb_dd68),
        ("spinlock@3ffbdc18", 0x3ffb_dc18),
        ("spinlock@3ffbdcb4", 0x3ffb_dcb4),
        ("s_ipc_isr_mux", 0x3ffb_dc24),
        // Global S32C1I helper lock used by esp_cpu_compare_and_set (L32R lit).
        ("s32c1i_ext_lock", 0x3ffb_eac0),
        // Candidate mux from APP phys snapshot (prior run).
        ("mux@3ffbe578", 0x3ffb_e578),
        ("mux@3ffc25d4", 0x3ffc_25d4),
        ("mux@3ffc2344", 0x3ffc_2344),
        ("mux@3ffc2da4", 0x3ffc_2da4),
    ] {
        let owner = machine.bus.read_u32(addr as u64).unwrap_or(0);
        let count = machine.bus.read_u32(addr as u64 + 4).unwrap_or(0);
        eprintln!("[diag] {name}@0x{addr:08x}: owner=0x{owner:08x} count=0x{count:x}");
    }
    // Scan a few words around APP SP for portMUX-looking pairs.
    if let Some(app) = machine.cpu_secondary.as_ref() {
        let sp = app.regs.read_logical(1) as u64;
        for base in [sp, sp + 0x40, sp + 0x80, 0x3ffb_eac0u64, 0x3ffc_2500u64] {
            for off in (0..32).step_by(8) {
                let o = machine.bus.read_u32(base + off).unwrap_or(0);
                let c = machine.bus.read_u32(base + off + 4).unwrap_or(0);
                if o == 0xb33f_ffff || o == 0 || o == 1 || (o & 0xffff_fffe) == 0 {
                    eprintln!(
                        "[diag] word@0x{:08x}: 0x{o:08x} 0x{c:08x}",
                        (base + off) as u32
                    );
                }
            }
        }
    }

    if let Ok(out) = std::process::Command::new("nm")
        .arg("-n")
        .arg(&elf)
        .output()
    {
        let text = String::from_utf8_lossy(&out.stdout);
        for (label, pc) in [("PRO", pro_pc), ("APP", app_pc)] {
            let mut prev = None;
            for line in text.lines() {
                let p: Vec<_> = line.split_whitespace().collect();
                if p.len() >= 3 && "TtWwAa".contains(p[1]) {
                    if let Ok(a) = u32::from_str_radix(p[0], 16) {
                        if a > pc {
                            if let Some((pa, pn)) = prev {
                                eprintln!("  {label} in 0x{pa:08x} {pn}");
                            }
                            break;
                        }
                        prev = Some((a, p[2].to_string()));
                    }
                }
            }
        }
    }
}
