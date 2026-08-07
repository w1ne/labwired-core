// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `labwired snapshot` subcommands: capture and inspect machine snapshots.

use crate::*;

pub(crate) fn run_snapshot(
    args: SnapshotArgs,
    plugins: &[&dyn labwired_core::plugin::ChipPlugin],
) -> ExitCode {
    match args.command {
        SnapshotCommands::Capture(a) => run_snapshot_capture(a, plugins),
    }
}

/// Drive a firmware mid-flight in a headless sim and write a runtime
/// snapshot blob. The playground reads the same blob to skip cold boot.
///
/// The `arduino-esp32` profile mirrors what
/// `WasmSimulator::install_arduino_esp32_quirks` plus `step_with_esp32_aids`
/// do on the web side — same configure_xtensa_esp32 bus, same handshake,
/// same thunk setup, same IPI bridge cadence — so the captured state will
/// resume bit-identically inside the browser. Thunk PCs are resolved from the
/// ELF symbol table (no hand-curated per-firmware address list).
pub(crate) fn run_snapshot_capture(
    args: SnapshotCaptureArgs,
    plugins: &[&dyn labwired_core::plugin::ChipPlugin],
) -> ExitCode {
    use labwired_core::bus::SystemBus;
    use labwired_core::peripherals::components::{Ssd1680Tricolor290, Uc8151dTricolor290};
    use labwired_core::peripherals::esp32::spi::Esp32Spi;
    use labwired_core::system::xtensa::configure_xtensa_esp32;
    use labwired_core::{Machine, SimulationError};
    use labwired_loader::{extract_arduino_esp32_thunks, load_elf_bytes};

    if args.profile != "arduino-esp32" {
        eprintln!(
            "error: unknown profile '{p}' — supported: 'arduino-esp32' (any Arduino-ESP32 ELF with symbols intact, auto-discovers thunk PCs)",
            p = args.profile
        );
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }

    let elf_bytes = match std::fs::read(&args.firmware) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: cannot read firmware ELF {:?}: {e}", args.firmware);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    // Bus + CPU — same configure_xtensa_esp32 that the WASM uses.
    let mut bus = SystemBus::new();
    let cpu = configure_xtensa_esp32(&mut bus);

    // Peripherals come from the board manifest, never hardcoded here. The
    // generic attach_esp32_external_devices factory wires every declared
    // external device (panel, etc.) onto its bus with the right model, CS and
    // DC pins. --system points at the board manifest (e.g. the ereader's
    // board.yaml declaring the SSD1680 e-paper on spi3, CS=GPIO5, DC=GPIO17).
    if let Some(sys_path) = &args.system {
        match labwired_config::SystemManifest::from_file(sys_path) {
            Ok(manifest) => {
                if let Err(e) = labwired_core::system::xtensa::attach_esp32_external_devices(
                    &mut bus, &manifest,
                ) {
                    eprintln!("error: attaching external devices from {sys_path:?}: {e}");
                    return ExitCode::from(EXIT_CONFIG_ERROR);
                }
                // Debugger register NAMES come from the chip YAML even though
                // the peripheral bank above is programmatic. Without this the
                // whole ESP32 bank inspects as `registers: []` no matter what
                // `debug_schema:` the chip declares. Never fatal — a missing
                // debugger convenience must not stop a capture.
                let chip_dir = sys_path
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."));
                match labwired_config::ChipDescriptor::resolve_with(
                    &manifest.chip,
                    chip_dir,
                    &crate::plugin_chip_yaml(plugins),
                ) {
                    Ok(chip) => bus.attach_debug_schemas(
                        &chip,
                        &labwired_core::system::builder::anchor_chip_path(&manifest, chip_dir),
                    ),
                    Err(e) => eprintln!(
                        "warning: cannot load chip descriptor {:?}: {e}; \
                         inspected registers will be unnamed",
                        manifest.chip
                    ),
                }
            }
            Err(e) => {
                eprintln!("error: cannot load system manifest {sys_path:?}: {e}");
                return ExitCode::from(EXIT_CONFIG_ERROR);
            }
        }
    } else {
        eprintln!(
            "warning: no --system manifest; no external peripherals attached \
             (firmware that drives a panel will not render)"
        );
    }
    // Enable wire-byte capture on spi3 for snapshot diagnostics (a capture
    // concern, not a device wiring concern).
    if let Some(spi3_idx) = bus.find_peripheral_index_by_name("spi3") {
        if let Some(any) = bus.peripherals[spi3_idx].dev.as_any_mut() {
            if let Some(spi3) = any.downcast_mut::<Esp32Spi>() {
                spi3.enable_byte_capture(65536);
            }
        }
    }
    bus.refresh_peripheral_index();

    // Drain UART TX so a sketch's own Serial output is recoverable. Without a
    // sink nothing reads the peripheral and this path could only ever report a
    // panel paint — a sketch that proves itself by printing had no evidence at
    // all. Written beside the snapshot as `<output>.uart.log`.
    let uart_tx = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    bus.attach_uart_tx_sink(uart_tx.clone(), false);

    let boxed: Box<dyn Cpu> = Box::new(cpu);
    let mut machine = Machine::new(boxed, bus);
    // Arduino-ESP32 sketches reach `xTaskCreatePinnedToCore(..., 1)`
    // for `loopTask` and others — without an APP_CPU to schedule onto,
    // FreeRTOS spins in `vListInsert` forever. Attach a secondary CPU
    // (PRID=0xABAB, halted at construction, released by
    // `ets_set_appcpu_boot_addr` during PRO_CPU boot).
    let cpu1 = labwired_core::cpu::xtensa_lx7::XtensaLx7::new_app_cpu();
    machine.cpu_secondary = Some(Box::new(cpu1));

    // Load firmware FIRST — load_firmware writes ELF segments into bus
    // memory, so any bytes we write before this risk being clobbered.
    // The handshake/header writes and `install_flash_thunk` (which patches
    // BREAK bytes into flash) must happen AFTER the ELF is in place.
    let program_image = match load_elf_bytes(&elf_bytes) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: load_elf_bytes: {e}");
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };
    if let Err(e) = machine.load_firmware(&program_image) {
        eprintln!("error: load_firmware: {e}");
        return ExitCode::from(EXIT_RUNTIME_ERROR);
    }
    // XtensaLx7::reset() leaves PC at the 0x40000400 BROM reset vector.
    // Skip BROM and jump straight to the ELF's app entry — same as WASM.
    // CHEAT(SKIP): bypasses the boot ROM and hand-seeds PC (SP seeded below).
    // See FIDELITY.md §C.
    machine.cpu.set_pc(program_image.entry_point as u32);

    // Resolve every Arduino-ESP32 symbol we know how to patch / thunk.
    // Empty for the reference firmware (stripped) — those fall back to hardcoded PCs.
    // The Arduino-ESP32 profile now lives in ONE place
    // (core/system/xtensa/arduino_esp32_profile.rs) so `labwired test` boots
    // these firmwares identically. It used to live only here, which is why the
    // declarative runner — the one that owns stimuli and assertions — could not
    // run a classic-ESP32 Arduino sketch at all.
    let symbol_addrs = extract_arduino_esp32_thunks(&elf_bytes);
    let profile = match labwired_core::system::xtensa::install_arduino_esp32_profile(
        &mut machine,
        symbol_addrs,
        program_image.entry_point as u32,
    ) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(EXIT_RUNTIME_ERROR);
        }
    };
    let symbol_addrs = profile.symbols.clone();
    eprintln!(
        "labwired-cli snapshot: installing {} thunks ({} resolved from ELF symbols)",
        profile.thunks_installed,
        symbol_addrs.len(),
    );

    eprintln!(
        "labwired-cli snapshot: stepping firmware to cycle {}",
        args.steps
    );
    // Instruction trace. Off unless asked for: attaching an observer forces
    // the interpreter path (a compiled block cannot emit per-step events), so
    // an always-on trace would silently tax every capture.
    let ring = args
        .trace_out
        .as_ref()
        .map(|_| std::sync::Arc::new(labwired_core::trace::RetiredRing::new(args.trace_last)));
    // The authoritative path owns its observers, so hand the trace ring to the
    // Machine instead of passing a list into every cpu.step call.
    if let Some(r) = &ring {
        machine.observers.push(r.clone());
    }
    if let Some(path) = &args.trace_out {
        eprintln!(
            "labwired-cli snapshot: tracing the last {} retired instructions to {}",
            args.trace_last,
            path.display()
        );
    }
    // Written on EVERY exit path, faults included — a trace that only survives
    // a clean run is useless for the case it exists to explain.
    let dump_trace = |ring: &Option<std::sync::Arc<labwired_core::trace::RetiredRing>>| {
        let (Some(ring), Some(path)) = (ring, args.trace_out.as_ref()) else {
            return;
        };
        let entries = ring.entries();
        let total = ring.total_retired();
        let payload = serde_json::json!({
            "total_retired": total,
            "kept": entries.len(),
            "dropped": total.saturating_sub(entries.len() as u64),
            "instructions": entries,
        });
        match std::fs::File::create(path) {
            Ok(f) => {
                if let Err(e) = serde_json::to_writer_pretty(f, &payload) {
                    eprintln!("error: failed to write {}: {e}", path.display());
                } else {
                    eprintln!(
                        "labwired-cli snapshot: wrote {} of {} retired instructions to {}",
                        entries.len(),
                        total,
                        path.display()
                    );
                }
            }
            Err(e) => eprintln!("error: failed to create {}: {e}", path.display()),
        }
    };

    let mut i: u64 = 0;
    let progress = args.progress_every;
    while i < args.steps {
        // `advance` rather than `step`: `step` discards the AdvanceReport, so a
        // firmware `simctl` verdict would be silently swallowed and this loop
        // would keep stepping a run the firmware had already ended.
        match machine.advance(labwired_core::AdvanceRequest::single()) {
            Ok(report) => {
                if let labwired_core::AdvanceStop::FirmwareExit { code } = report.stop {
                    eprintln!(
                        "[firmware] {} (step {i})",
                        crate::firmware_exit_message(code)
                    );
                    break;
                }
            }
            Err(SimulationError::BreakpointHit(_)) => {}
            Err(e) => {
                eprintln!(
                    "error: sim step at cycle {i} pc=0x{:08x}: {e}",
                    machine.cpu.get_pc()
                );
                dump_trace(&ring);
                return ExitCode::from(EXIT_RUNTIME_ERROR);
            }
        }
        // Secondary-CPU release, secondary stepping and peripheral ticking
        // all happen inside Machine::step. Doing them here as well is how
        // this loop drifted from every other runner in the first place.
        i += 1;
        if progress > 0 && i.is_multiple_of(progress) {
            let cpu1_state = match machine.cpu_secondary.as_ref() {
                Some(cpu1) => format!("  cpu1=0x{:08x}", cpu1.get_pc()),
                None => String::new(),
            };
            eprintln!(
                "  step {i:>10}  pc=0x{:08x}{cpu1_state}",
                machine.cpu.get_pc()
            );
            // Optional DC7 debug: dump vListInsert state on spin. Set
            // LABWIRED_DEBUG_LIST=1 to enable. Shows cpu intlevel,
            // xTaskQueueMutex state, pxList walk, and newItem state.
            if std::env::var("LABWIRED_DEBUG_LIST").is_ok() {
                eprintln!(
                    "    cpu0 intlevel={} a0=0x{:08x} a1=0x{:08x}",
                    machine.cpu.intlevel(),
                    machine.cpu.get_register(0),
                    machine.cpu.get_register(1)
                );
                let mux_owner = machine.bus.read_u32(0x3ffbf3b8).unwrap_or(0xDEAD);
                let mux_count = machine.bus.read_u32(0x3ffbf3bc).unwrap_or(0xDEAD);
                eprintln!("    xTaskQueueMutex.owner=0x{mux_owner:08x} .count={mux_count}");
                if let Some(cpu1) = machine.cpu_secondary.as_ref() {
                    eprintln!(
                        "    cpu1 intlevel={} a0=0x{:08x} a1=0x{:08x}",
                        cpu1.intlevel(),
                        cpu1.get_register(0),
                        cpu1.get_register(1)
                    );
                }
                let px_list = machine.cpu.get_register(2);
                let r = |off: u32| {
                    machine
                        .bus
                        .read_u32((px_list + off) as u64)
                        .unwrap_or(0xDEAD)
                };
                eprintln!(
                    "    cpu0 pxList=0x{px_list:08x} num={} idx=0x{:08x} end.val=0x{:08x} end.next=0x{:08x} end.prev=0x{:08x}",
                    r(0), r(4), r(8), r(12), r(16)
                );
                if let Some(cpu1) = machine.cpu_secondary.as_ref() {
                    let px_list1 = cpu1.get_register(2);
                    let r1 = |off: u32| {
                        machine
                            .bus
                            .read_u32((px_list1 + off) as u64)
                            .unwrap_or(0xDEAD)
                    };
                    eprintln!(
                        "    cpu1 pxList=0x{px_list1:08x} num={} idx=0x{:08x} end.val=0x{:08x} end.next=0x{:08x} end.prev=0x{:08x}",
                        r1(0), r1(4), r1(8), r1(12), r1(16)
                    );
                }
                let mut iter = r(12);
                let end_addr = px_list + 8;
                for hop in 0..6 {
                    if iter == end_addr {
                        eprintln!("      [hop {hop}] -> xListEnd (terminator)");
                        break;
                    }
                    let item_next = machine.bus.read_u32((iter + 4) as u64).unwrap_or(0xDEAD);
                    let item_val = machine.bus.read_u32(iter as u64).unwrap_or(0xDEAD);
                    eprintln!("      [hop {hop}] item=0x{iter:08x} val=0x{item_val:08x} next=0x{item_next:08x}");
                    iter = item_next;
                }
                let new_item = machine.cpu.get_register(3);
                let ri = |off: u32| {
                    machine
                        .bus
                        .read_u32((new_item + off) as u64)
                        .unwrap_or(0xDEAD)
                };
                eprintln!(
                    "    cpu0 newItem=0x{new_item:08x} item.val=0x{:08x} item.next=0x{:08x} item.prev=0x{:08x} item.owner=0x{:08x}",
                    ri(0), ri(4), ri(8), ri(12)
                );
            }
        }
    }

    // Sanity-check the captured state — we expect the panel to have been
    // driven through at least one refresh cycle by the time the snapshot
    // lands. Print this so the operator can tell "yes, this snapshot is
    // post-paint" without re-running the playground.
    if let Some(idx) = machine.bus.find_peripheral_index_by_name("spi3") {
        if let Some(any) = machine.bus.peripherals[idx].dev.as_any() {
            if let Some(spi3) = any.downcast_ref::<Esp32Spi>() {
                // Diagnostic: dump the full captured wire stream when asked, so
                // we can inspect the 0x24/0x26 RAM-write payloads end-to-end.
                if let Ok(path) = std::env::var("LABWIRED_DUMP_SPI") {
                    let _ = std::fs::write(&path, spi3.captured_bytes());
                    eprintln!(
                        "labwired-cli snapshot: dumped {} captured spi3 bytes to {path}",
                        spi3.captured_bytes().len()
                    );
                }
                eprintln!(
                    "labwired-cli snapshot: spi3 transactions={}",
                    spi3.transactions(),
                );
                let cap = spi3.captured_bytes();
                if !cap.is_empty() {
                    let head_n = cap.len().min(120);
                    let head_hex: Vec<String> =
                        cap[..head_n].iter().map(|b| format!("{b:02x}")).collect();
                    eprintln!(
                        "labwired-cli snapshot: first {head_n} spi3 bytes: {}",
                        head_hex.join(" ")
                    );
                    if cap.len() > 240 {
                        let tail = &cap[cap.len() - 120..];
                        let tail_hex: Vec<String> =
                            tail.iter().map(|b| format!("{b:02x}")).collect();
                        eprintln!(
                            "labwired-cli snapshot: last 120 spi3 bytes: {}",
                            tail_hex.join(" ")
                        );
                    }
                }
                for attached in &spi3.attached_devices {
                    if let Some(panel_any) = attached.as_any() {
                        if let Some(panel) = panel_any.downcast_ref::<Ssd1680Tricolor290>() {
                            let bp = panel.black_plane();
                            let non_ff = bp.iter().filter(|&&b| b != 0xFF).count();
                            eprintln!(
                                "labwired-cli snapshot: panel (ssd1680) state — refresh_generation={}, power_on={}, black-plane non-FF bytes={}/{}",
                                panel.refresh_generation(),
                                panel.power_on(),
                                non_ff,
                                bp.len(),
                            );
                        } else if let Some(panel) = panel_any
                            .downcast_ref::<labwired_core::peripherals::components::Ili9341>(
                        ) {
                            // An RGB565 TFT has no e-paper "refresh" — the
                            // frame memory IS the screen — so the evidence is
                            // DISPON plus how much of the framebuffer the
                            // firmware actually wrote. Without this line the
                            // panel produced no run evidence at all: a `display`
                            // oracle clause could not resolve, and a lab could
                            // only assert that `tft.begin()` returned, which is
                            // a host-side value the driver tracks itself and
                            // would read the same with no panel on the bus.
                            //
                            // `refresh_generation` is reported as 1 once the
                            // display is on and pixels exist, so one oracle
                            // shape covers both panel families.
                            let fb = panel.framebuffer();
                            let painted = fb.iter().filter(|&&b| b != 0x00).count();
                            let generation = u32::from(panel.display_on() && painted > 0);
                            let (w, h) = panel.dimensions();
                            // The most common non-black pixel, so the line says
                            // WHAT was drawn and not merely that something was.
                            // "10176 bytes changed" cannot be checked against a
                            // photo of the real panel; "top colour 0x07E0"
                            // (RGB565 green) can.
                            let mut counts: std::collections::HashMap<u16, usize> =
                                std::collections::HashMap::new();
                            for px in fb.chunks_exact(2) {
                                let v = u16::from_be_bytes([px[0], px[1]]);
                                if v != 0 {
                                    *counts.entry(v).or_default() += 1;
                                }
                            }
                            let top = counts
                                .iter()
                                .max_by_key(|&(_, n)| *n)
                                .map(|(v, n)| format!("0x{v:04X} x{n}"))
                                .unwrap_or_else(|| "none".to_string());
                            eprintln!(
                                "labwired-cli snapshot: panel (ili9341) state — refresh_generation={}, display_on={}, painted bytes={}/{}, {}x{}, top colour {}",
                                generation,
                                panel.display_on(),
                                painted,
                                fb.len(),
                                w,
                                h,
                                top,
                            );
                        } else if let Some(panel) = panel_any.downcast_ref::<Uc8151dTricolor290>() {
                            let bp = panel.black_plane();
                            let non_ff = bp.iter().filter(|&&b| b != 0xFF).count();
                            let rp = panel.red_plane();
                            let non_ff_red = rp.iter().filter(|&&b| b != 0xFF).count();
                            eprintln!(
                                "labwired-cli snapshot: panel (uc8151d) state — refresh_generation={}, power_on={}, black-plane non-FF bytes={}/{}, red-plane non-FF bytes={}/{}",
                                panel.refresh_generation(),
                                panel.power_on(),
                                non_ff,
                                bp.len(),
                                non_ff_red,
                                rp.len(),
                            );
                            // Render the panel as a PPM next to the
                            // snapshot output so an operator can visually
                            // confirm "yes, this looks like the real-HW
                            // panel image" before shipping the snapshot.
                            let (w, h) = panel.dimensions();
                            let stride = w / 8;
                            let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
                            for y in 0..h {
                                for x in 0..w {
                                    let idx = y * stride + x / 8;
                                    let bit = 7 - (x % 8);
                                    let black_bit = (bp[idx] >> bit) & 1;
                                    let red_bit = (rp[idx] >> bit) & 1;
                                    let (r, g, b) = if red_bit == 0 {
                                        (220u8, 30u8, 40u8)
                                    } else if black_bit == 0 {
                                        (0u8, 0u8, 0u8)
                                    } else {
                                        (245u8, 245u8, 240u8)
                                    };
                                    ppm.extend_from_slice(&[r, g, b]);
                                }
                            }
                            let ppm_path = args.output.with_extension("ppm");
                            if std::fs::write(&ppm_path, &ppm).is_ok() {
                                eprintln!(
                                    "labwired-cli snapshot: panel PPM written to {}",
                                    ppm_path.display()
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    dump_trace(&ring);

    // Ask before taking. `Cpu::runtime_snapshot` no longer panics on the arches
    // that do not implement it (a panic in wasm is a trap, and a trap leaks
    // wasm-bindgen's borrow guard — see the note on the trait), so an ungated
    // call here would no longer fail loudly: it would write a well-formed file
    // whose CPU blob is EMPTY. A resume from that blob restores no registers at
    // all, and nothing downstream can tell it apart from a real capture. Refuse
    // instead — a missing snapshot is recoverable, a lying one is not.
    if !machine.cpu.supports_runtime_snapshot() {
        eprintln!(
            "error: this CPU has no runtime-snapshot implementation, so no resumable \
             snapshot can be captured for it (supported: RISC-V, Xtensa LX7)."
        );
        return ExitCode::from(EXIT_RUNTIME_ERROR);
    }

    let snap = machine.take_runtime_snapshot();
    let bytes = snap.to_bytes();

    if let Some(parent) = args.output.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&args.output, &bytes) {
        eprintln!("error: write {:?}: {e}", args.output);
        return ExitCode::from(EXIT_RUNTIME_ERROR);
    }

    // Persist whatever the sketch printed, and report the byte count so a caller
    // parsing stderr knows a log exists without stat-ing for it.
    {
        let bytes = uart_tx.lock().map(|b| b.clone()).unwrap_or_default();
        let uart_path = args.output.with_extension("uart.log");
        match std::fs::write(&uart_path, &bytes) {
            Ok(()) => eprintln!(
                "labwired-cli snapshot: uart {} bytes -> {:?}",
                bytes.len(),
                uart_path
            ),
            Err(e) => eprintln!("labwired-cli snapshot: warn: write {uart_path:?}: {e}"),
        }
    }

    // The universal final-state inspect block — the SAME `machine.inspect()`
    // payload the `test` path puts in result.json, written beside the snapshot
    // as `<output>.inspect.json`.
    //
    // Without this the profile path's only evidence was an e-paper refresh
    // scraped out of the stderr text above, so a classic-ESP32 sketch that
    // reported through GPIO — or through any peripheral that is not a panel on
    // spi3 — produced a clean run with nothing to verify against, and an oracle
    // asserting pin state could never pass however correct the firmware was.
    // Two run paths, two different answers to "what did the hardware do";
    // this makes it one.
    {
        let inspect_block = machine.inspect(
            None,
            &labwired_core::inspect::InspectOpts {
                include_bytes: false,
                peripheral: None,
            },
        );
        let inspect_path = args.output.with_extension("inspect.json");
        match serde_json::to_vec(&inspect_block) {
            Ok(json) => match std::fs::write(&inspect_path, &json) {
                Ok(()) => eprintln!(
                    "labwired-cli snapshot: inspect {} peripheral(s), {} external device(s), \
                     {}/{} registers model-backed -> {:?}",
                    inspect_block.peripherals.len(),
                    inspect_block.devices.len(),
                    inspect_block
                        .peripherals
                        .iter()
                        .flat_map(|p| &p.registers)
                        .filter(|r| r.value.is_some())
                        .count(),
                    inspect_block
                        .peripherals
                        .iter()
                        .map(|p| p.registers.len())
                        .sum::<usize>(),
                    inspect_path
                ),
                Err(e) => eprintln!("labwired-cli snapshot: warn: write {inspect_path:?}: {e}"),
            },
            Err(e) => eprintln!("labwired-cli snapshot: warn: encode inspect block: {e}"),
        }
    }

    eprintln!(
        "labwired-cli snapshot: wrote {} bytes to {:?} (pc=0x{:08x} after {} cycles)",
        bytes.len(),
        args.output,
        machine.cpu.get_pc(),
        args.steps,
    );
    // Phase 3.2 JIT pilot (issue #124): report block hit count if the
    // build was compiled with `--features jit-core`. Without the feature
    // the trait default returns 0 and this line is harmless.
    let jit_hits = machine.cpu.jit_hit_count();
    if jit_hits > 0 {
        eprintln!("labwired-cli snapshot: jit block hits: {jit_hits}");
    }
    ExitCode::from(EXIT_PASS)
}
