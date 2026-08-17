// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `labwired run` + interactive (gdb/dap) drivers across ARM / RISC-V / Xtensa.

use crate::artifacts::{write_interactive_snapshot, InteractiveSnapshotInputs};
use crate::*;

/// Export every attached parallel-panel framebuffer, if `--display-out <path>`
/// was given: a binary PPM per panel (`<path>` for the first, `<path>.<id>`
/// for any others) plus a luma ASCII map on stderr.
///
/// Evidence, not decoration: a lit-pixel count alone cannot tell "the panel
/// painted the frame" from "the panel painted noise", so the run prints
/// something a human can compare against the firmware's own thumbnail.
/// Non-fatal — a write error never changes the run's exit code.
pub(crate) fn export_display_if_requested(
    display_out: &Option<PathBuf>,
    bus: &labwired_core::bus::SystemBus,
) {
    let Some(path) = display_out else {
        return;
    };
    if bus.ili9341_parallel.is_empty() {
        eprintln!("labwired-cli run: --display-out given but no parallel panel is attached");
        return;
    }
    for (n, panel) in bus.ili9341_parallel.iter().enumerate() {
        let (w, h) = panel.logical_dimensions();
        let fb = panel.oriented_framebuffer();
        let ink = fb.iter().filter(|&&b| b != 0).count();
        eprintln!(
            "labwired-cli run: panel '{}' {w}x{h} display_on={} ink_bytes={ink}/{}",
            panel.id(),
            panel.display_on(),
            fb.len(),
        );

        // Luma ASCII map, 64 columns wide, aspect-corrected for a terminal.
        const COLS: usize = 64;
        const ROWS: usize = 24;
        const RAMP: &[u8] = b" .:-=+*#%@";
        for r in 0..ROWS {
            let sy = r * h / ROWS;
            let mut line = String::with_capacity(COLS);
            for c in 0..COLS {
                let sx = c * w / COLS;
                let i = (sy * w + sx) * 2;
                let px = u16::from_be_bytes([fb[i], fb[i + 1]]);
                let (red, green, blue) = (
                    ((px >> 11) & 0x1F) as u32 * 255 / 31,
                    ((px >> 5) & 0x3F) as u32 * 255 / 63,
                    (px & 0x1F) as u32 * 255 / 31,
                );
                let luma = (red * 77 + green * 150 + blue * 29) >> 8;
                line.push(RAMP[(luma as usize * (RAMP.len() - 1)) / 255] as char);
            }
            eprintln!("|{line}|");
        }

        // RGB888 binary PPM.
        let out_path = if n == 0 {
            path.clone()
        } else {
            let mut p = path.clone().into_os_string();
            p.push(format!(".{}", panel.id()));
            PathBuf::from(p)
        };
        let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
        for px in fb.chunks_exact(2) {
            let v = u16::from_be_bytes([px[0], px[1]]);
            ppm.push((((v >> 11) & 0x1F) as u32 * 255 / 31) as u8);
            ppm.push((((v >> 5) & 0x3F) as u32 * 255 / 63) as u8);
            ppm.push(((v & 0x1F) as u32 * 255 / 31) as u8);
        }
        match std::fs::write(&out_path, &ppm) {
            Ok(()) => eprintln!("labwired-cli run: panel image -> {out_path:?}"),
            Err(e) => eprintln!("error: cannot write --display-out {out_path:?}: {e}"),
        }
    }
}

/// Export the bus trace (logic analyzer) captured by `bus`, if
/// `--bus-trace-out <path>` was given. Dispatches by extension: `.json`
/// writes the raw event list, anything else writes VCD (GTKWave / PulseView
/// / Saleae / sigrok). Non-fatal: a write error is reported on stderr but
/// does not change the run's exit code, since the simulation itself already
/// completed.
pub(crate) fn export_bus_trace_if_requested(
    bus_trace_out: &Option<PathBuf>,
    bus: &labwired_core::bus::SystemBus,
) {
    let Some(path) = bus_trace_out else {
        return;
    };
    let events = bus.bus_trace_snapshot();
    let file = match std::fs::File::create(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: cannot create bus-trace-out file {path:?}: {e}");
            return;
        }
    };
    let is_json = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("json"))
        .unwrap_or(false);
    let result = if is_json {
        labwired_cli::bus_vcd::write_bus_trace_json(&events, file)
    } else {
        labwired_cli::bus_vcd::write_bus_trace_vcd(&events, file)
    };
    match result {
        Ok(()) => eprintln!(
            "labwired-cli run: bus trace ({} events) -> {path:?}",
            events.len()
        ),
        Err(e) => eprintln!("error: failed to write bus-trace-out {path:?}: {e}"),
    }
}

pub(crate) fn run_firmware_riscv(
    args: RunArgs,
    _chip_yaml: String,
    plugins: &[&dyn labwired_core::plugin::ChipPlugin],
) -> ExitCode {
    use labwired_core::bus::SystemBus;

    let chip = match labwired_config::ChipDescriptor::from_file(&args.chip) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: cannot parse chip YAML {:?}: {e}", args.chip);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    // Minimal system manifest: no external devices, no extra peripherals.
    // All peripherals come from the chip descriptor.
    let manifest = labwired_config::SystemManifest {
        parts: Vec::new(),
        schema_version: "1.0".to_string(),
        name: chip.name.clone(),
        chip: args.chip.to_string_lossy().into_owned(),
        cpu_hz: None,
        memory_overrides: Default::default(),
        external_devices: vec![],
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        board_io: vec![],
        debug_uart: None,
        wifi_ap: None,
        peripherals: vec![],
        walk_deleted: Some(false),
    };

    // Two-station WiFi run (env LABWIRED_WIFI_DUAL): boot two C3 instances with
    // distinct MACs onto the shared VirtualWifi medium so they associate, get
    // distinct DHCP leases, and exchange traffic over one virtual AP.
    if args.rom_boot && std::env::var("LABWIRED_WIFI_DUAL").is_ok() {
        return run_two_c3_wifi(&args, &chip, &manifest, plugins);
    }

    // Two-node BLE run (env LABWIRED_BLE_DUAL): boot two C3 instances with
    // different firmware onto the shared BLE air, so one advertises while the
    // other scans and the scanner's stack sees the advertiser's PDU.
    if args.rom_boot && std::env::var("LABWIRED_BLE_DUAL").is_ok() {
        return crate::run_two_c3_ble(&args, &chip, &manifest, plugins);
    }

    // Single-station WiFi run (env LABWIRED_WIFI_SOLO): one C3 on the shared
    // VirtualWifi medium — associates, gets a DHCP lease, and reaches the AP's
    // DHCP + HTTP servers (the LBC3.1 stats-device demo). Uses its own minimal
    // step loop like the dual path; bolting medium mode onto the standard run
    // loop below does not keep the MAC resident (auth never completes).
    if args.rom_boot && std::env::var("LABWIRED_WIFI_SOLO").is_ok() {
        return crate::run_one_c3_wifi(&args, &chip, &manifest, plugins);
    }

    let mut bus = match SystemBus::from_config_with_plugins(&chip, &manifest, plugins) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: failed to build system bus: {e:#}");
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let program = match labwired_loader::load_elf(&args.firmware) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: cannot load ELF {:?}: {e}", args.firmware);
            return ExitCode::from(EXIT_RUNTIME_ERROR);
        }
    };

    let mut machine = if args.rom_boot {
        match build_c3_rom_boot_machine(bus, None) {
            Ok(m) => m,
            Err(code) => return code,
        }
    } else {
        let cpu = labwired_core::system::riscv::configure_riscv(&mut bus);
        let mut machine = labwired_core::Machine::new(cpu, bus);
        if let Err(e) = machine.load_firmware(&program) {
            eprintln!("error: firmware load failed: {e}");
            return ExitCode::from(EXIT_RUNTIME_ERROR);
        }

        // Fast-boot skips the ROM/2nd-stage bootloader that normally sets the
        // stack pointer before jumping to the app, so SP=0 and the app's first
        // prologue store faults near 0xffffffff. Seed SP at the top of DRAM
        // (16-byte aligned, RISC-V ABI) so real IDF apps can boot.
        let sp_top = (chip.ram.base + chip.ram.size) as u32;
        machine.cpu.set_sp(sp_top & !0xF);
        machine
    };

    // Keep the RISC-V fast-boot path observable through the same UART capture
    // mechanism as ARM/Xtensa. This is an output transport, not a timing or
    // CPU-model shortcut: the C3 UART peripheral still produces every byte.
    let uart_sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    machine.bus.attach_uart_tx_sink(uart_sink, true);

    let break_at: Vec<u32> = args
        .break_at
        .iter()
        .filter_map(|s| u32::from_str_radix(s.trim_start_matches("0x"), 16).ok())
        .collect();
    let mut break_hit = vec![false; break_at.len()];
    let limit = args.max_steps.unwrap_or(u64::MAX);
    // Recent-PC trail for boot debugging — only maintained when --break-at is in
    // use, so the normal hot loop pays nothing.
    let debug = !break_at.is_empty();
    // Executable address windows for C3 (ROM, IRAM, flash IROM XIP). A PC
    // outside all of these means a bad jump (truncated pointer, garbage return
    // address); trap it immediately so the trail still shows the jumper instead
    // of 64 instructions of slide through unmapped memory.
    let is_exec = |pc: u32| -> bool {
        (0x4000_0000..0x4006_0000).contains(&pc)      // mask ROM
            || (0x4037_0000..0x403E_0000).contains(&pc) // IRAM
            || (0x4200_0000..0x4400_0000).contains(&pc) // flash IROM (XIP)
    };
    let trail_cap = 600;
    let mut recent = std::collections::VecDeque::with_capacity(trail_cap + 1);
    // WiFi bridge (env-gated LABWIRED_WIFI_BRIDGE): inject an OPEN beacon for
    // "labwired-ap" into the real MAC's RX ring periodically after the MAC is
    // up, so the driver's scan finds the AP and proceeds to auth/assoc — the
    // first comms milestone over the real MAC. Repeated injection covers the
    // scan's channel hopping. A frame-level VirtualAp will subsume this.
    let bridge = std::env::var("LABWIRED_WIFI_BRIDGE").is_ok()
        || std::env::var("LABWIRED_WIFI_BRIDGE_RE").is_ok();
    let dhcp_trace = std::env::var("LABWIRED_DHCP_TRACE").is_ok();

    // ── Non-instrumented hot path: batch through Machine::run ────────────────
    // When nothing needs per-instruction visibility (no --break-at, no WiFi
    // bridge, no DHCP trace), run in batches through `Machine::run` so the
    // RV32IMC wasm-JIT can engage (it only compiles multi-instruction batches,
    // and its correctness gate refuses to run when observers/breakpoints/etc.
    // are present). The debug / bridge / dhcp paths below keep single-stepping
    // via `machine.step()`, which pins the batch to one instruction and so keeps
    // the JIT correctly OFF — preserving every existing break/halt-trail/inject
    // behavior. Byte-identity of the batched (JIT-on) path to the single-step
    // interpreter is proven by tests/riscv_jit_c3_oled_differential.rs.
    if !debug && !bridge && !dhcp_trace {
        return run_firmware_riscv_batched(machine, &args, limit);
    }

    // `--batched` is an assertion that the run took the batched path, and the
    // instrumentation below deliberately does not. Failing here is the point:
    // the alternative is a caller measuring the single-step interpreter while
    // believing it measured the batched orchestration.
    if args.batched {
        eprintln!(
            "error: --batched cannot be honoured together with per-instruction \
             instrumentation (--break-at / LABWIRED_WIFI_BRIDGE / \
             LABWIRED_DHCP_TRACE), which pins the CPU quantum to one instruction",
        );
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }

    // Find the behavioral wifi_mac model by type (the declarative chip-yaml
    // "wifi_mac" shares the name; routing uses ours via greatest-start-wins, but
    // name lookup would return the declarative one).
    let wifi_mac_idx = machine.bus.peripherals.iter().position(|p| {
        p.dev
            .as_any()
            .and_then(|a| {
                a.downcast_ref::<labwired_core::peripherals::esp32c3::wifi_mac::Esp32c3WifiMac>()
            })
            .is_some()
    });
    let mut next_beacon_at: u64 = 14_000_000;
    // 802.11 sequence counter for AP→STA frames: real APs increment it, and the
    // receiver dedups by (transmitter, seq) — without it, every frame after the
    // first (all seq 0) is dropped as a retransmission.
    let mut ap_seq: u16 = 0;
    // Stamp the next sequence number into a frame's seq-control field (bytes
    // 22..23 = seq<<4 | frag) and queue it for RX injection.
    macro_rules! stamp_seq {
        ($fr:expr) => {{
            if $fr.len() >= 24 {
                let sc = (ap_seq & 0xFFF) << 4;
                $fr[22] = sc as u8;
                $fr[23] = (sc >> 8) as u8;
                ap_seq = ap_seq.wrapping_add(1);
            }
        }};
    }
    // Beacons go on the back of the RX queue (best-effort, droppable).
    macro_rules! inject {
        ($mac:expr, $frame:expr) => {{
            let mut fr = $frame;
            stamp_seq!(fr);
            $mac.queue_rx_frame(fr);
        }};
    }
    // Unicast responses jump to the FRONT so they reach the driver inside its
    // per-state timeout window rather than queuing behind backlogged beacons.
    macro_rules! inject_priority {
        ($mac:expr, $frame:expr) => {{
            let mut fr = $frame;
            stamp_seq!(fr);
            $mac.queue_rx_priority(fr);
        }};
    }
    if bridge {
        eprintln!("[bridge] on; wifi_mac_idx={wifi_mac_idx:?}");
    }

    for i in 0..limit {
        // Periodic beacon so the STA's scan finds the AP (real APs beacon ~always).
        if bridge && i >= next_beacon_at {
            next_beacon_at = i + 2_000_000;
            if let Some(idx) = wifi_mac_idx {
                if let Some(any) = machine.bus.peripherals[idx].dev.as_any_mut() {
                    if let Some(mac) = any
                        .downcast_mut::<labwired_core::peripherals::esp32c3::wifi_mac::Esp32c3WifiMac>(
                        )
                    {
                        // Only beacon when the RX backlog is drained, so periodic
                        // beacons never delay a pending unicast response.
                        if mac.pending_rx_len() == 0 {
                            for ch in [1u8, 6, 11] {
                                inject!(mac, build_open_beacon("labwired-ap", ch));
                            }
                        }
                    }
                }
            }
        }
        // Event-driven virtual AP: drain everything the STA transmits and answer
        // each frame by type (probe/auth/assoc → mgmt resp, DHCP → DORA, ARP →
        // reply for the gateway). Responding to the STA's actual TX — rather than
        // blind-injecting on a timer — keeps association + DHCP deterministic and
        // lets a connected STA re-auth cleanly. Drained often so responses land
        // inside the driver's per-state timeout windows.
        if bridge && i % 20_000 == 0 {
            if let Some(idx) = wifi_mac_idx {
                if let Some(any) = machine.bus.peripherals[idx].dev.as_any_mut() {
                    if let Some(mac) = any
                        .downcast_mut::<labwired_core::peripherals::esp32c3::wifi_mac::Esp32c3WifiMac>(
                        )
                    {
                        let txs = mac.take_tx_frames();
                        for tx in txs {
                            if std::env::var("LABWIRED_BRIDGE_TRACE").is_ok() {
                                eprintln!("[bridge] STA TX {} at step {i}", tx_kind(&tx));
                            }
                            for (reply, label) in ap_respond(&tx) {
                                inject_priority!(mac, reply);
                                eprintln!("[bridge] {label} at step {i}");
                            }
                        }
                    }
                }
            }
        }
        let pc = machine.cpu.get_pc();
        // DHCP function-entry watch (env LABWIRED_DHCP_TRACE): logs each time the
        // CPU enters a key lwIP DHCP routine, to see whether the 500ms fine timer
        // fires (dhcp_fine_tmr/dhcp_timeout) and whether dhcp_bind is reached.
        if dhcp_trace {
            let name = match pc {
                0x42059298 => Some("dhcp_check"),
                0x420592fc => Some("dhcp_bind"),
                0x4205a186 => Some("dhcp_timeout"),
                0x4205a216 => Some("dhcp_fine_tmr"),
                0x420598c8 => Some("dhcp_handle_ack"),
                0x42059a04 => Some("dhcp_recv"),
                _ => None,
            };
            if let Some(n) = name {
                eprintln!("[dhcp] {n} at step {i}");
            }
        }
        if debug {
            if recent.len() == trail_cap {
                recent.pop_front();
            }
            recent.push_back(pc);
            if i > 0 && !is_exec(pc) {
                let c = &machine.cpu;
                eprintln!(
                    "[badjump] step {i}: PC entered non-exec region {pc:#010x} \
                     ra={:#010x} sp={:#010x} a0={:#010x}",
                    c.x[1], c.x[2], c.x[10]
                );
                let trail: Vec<String> = recent.iter().map(|p| format!("{p:#010x}")).collect();
                eprintln!("[trail] {}", trail.join(" -> "));
                break;
            }
        }
        if let Some(bi) = break_at.iter().position(|&b| b == pc) {
            if !break_hit[bi] {
                break_hit[bi] = true;
                let c = &machine.cpu;
                eprintln!(
                    "[break] step {i} pc={pc:#010x} ra={:#010x} sp={:#010x} a0={:#010x}",
                    c.x[1], c.x[2], c.x[10]
                );
            }
        }
        if debug && i > 0 && i % 20_000_000 == 0 {
            eprintln!("[progress] step {i} pc={pc:#010x}");
        }
        // `advance` rather than `step`: `step` throws away the AdvanceReport,
        // and a firmware-authored verdict lives only in that report. The
        // request is the one `step` issues, so stepping is unchanged.
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
            Err(e) => {
                // Surface the halt (was a silent debug log): the fault PC + reason is
                // the key signal when bringing real firmware up on the sim.
                tracing::debug!("labwired-riscv: step {i} pc={pc:#010x} halt: {e}");
                if !break_at.is_empty() {
                    eprintln!("[halt] step {i} pc={pc:#010x} err={e}");
                    let trail: Vec<String> = recent.iter().map(|p| format!("{p:#010x}")).collect();
                    eprintln!("[trail] {}", trail.join(" -> "));
                }
                break;
            }
        }
    }

    export_bus_trace_if_requested(&args.bus_trace_out, &machine.bus);
    export_display_if_requested(&args.display_out, &machine.bus);
    ExitCode::from(EXIT_PASS)
}

/// The RISC-V (ESP32-C3) non-instrumented hot path: run in batches through
/// `Machine::run` so the RV32IMC wasm-JIT (core feature `jit`, CLI feature
/// `jit-core`) can engage on multi-instruction batches. Only reached when no
/// per-instruction instrumentation (--break-at / WiFi bridge / DHCP trace) is
/// active, so the JIT's correctness gate (no observers, no push tap, not
/// cycle-accurate) is satisfied and compiled blocks retire atomically.
///
/// The JIT is byte-identical to the single-step interpreter — proven on the
/// real C3 OLED lab by tests/riscv_jit_c3_oled_differential.rs. It is default-ON
/// here; set `LABWIRED_RISCV_JIT=0` to force the interpreter (the escape hatch).
/// Preserves the single-step path's semantics: EXIT_PASS on completion, a halt
/// ends the run, and the bus trace is exported if requested.
fn run_firmware_riscv_batched(
    mut machine: labwired_core::Machine<labwired_core::cpu::RiscV>,
    args: &RunArgs,
    limit: u64,
) -> ExitCode {
    use labwired_core::bus::RECOMMENDED_TICK_INTERVAL;
    use labwired_core::DebugControl;

    // Escape hatch: LABWIRED_RISCV_JIT=0 forces the interpreter (default on).
    let jit_on = std::env::var("LABWIRED_RISCV_JIT").as_deref() != Ok("0");

    // The C3 is walk-deletable at rom-boot: its peripherals are scheduler-driven,
    // so batching at RECOMMENDED_TICK_INTERVAL is byte-identical to
    // interval-1 while giving the JIT a batch window wide enough to retire whole
    // basic blocks between peripheral ticks (see the differential gate). Set on
    // BOTH machine.config and machine.bus.config, exactly as the gate does.
    machine.config.peripheral_tick_interval = RECOMMENDED_TICK_INTERVAL;
    machine.bus.config.peripheral_tick_interval = RECOMMENDED_TICK_INTERVAL;
    machine.config.riscv_jit_enabled = jit_on;
    machine.bus.config.riscv_jit_enabled = jit_on;

    // Chunk the run so a u64::MAX `limit` (no --max-steps) stays bounded per
    // `Machine::run` call. `Machine::run` batches internally at the tick
    // interval; we only cap the total instruction budget here.
    const CHUNK: u32 = 4_000_000;
    let mut ran: u64 = 0;
    while ran < limit {
        let n = if limit == u64::MAX {
            CHUNK
        } else {
            CHUNK.min((limit - ran) as u32)
        };
        let before = machine.step_profile().cpu_instructions;
        match machine.run(Some(n)) {
            Ok(_) => {}
            Err(e) => {
                // A halt is the normal end of a fixture run; the fault PC/reason
                // is only surfaced on the debug (--break-at) path.
                tracing::debug!("labwired-riscv (batched): halt: {e}");
                break;
            }
        }
        let delta = machine.step_profile().cpu_instructions - before;
        ran += delta;
        // No forward progress (idle with no fast-forward budget): stop rather
        // than spin re-issuing empty batches up to `limit`.
        if delta == 0 {
            break;
        }
    }

    // Opt-in non-vacuity / diagnostic: prove the JIT actually compiled and ran
    // hot blocks on this run (LABWIRED_JIT_STATS=1). Only meaningful in a
    // `jit-core` build; the accessor does not exist otherwise.
    #[cfg(feature = "jit-core")]
    if std::env::var("LABWIRED_JIT_STATS").is_ok() {
        match machine.cpu.jit_stats() {
            Some(s) => eprintln!(
                "[jit-stats] compiled={} block_runs={} block_instrs={} interpreted={}",
                s.compiled, s.block_runs, s.block_instrs, s.interpreted
            ),
            None => eprintln!("[jit-stats] JIT engine never created (interpreter-only run)"),
        }
    }

    // Same proof-of-path line the ARM batched loop prints, and for the same
    // reason: `--batched` on RISC-V is an assertion about which loop ran, so it
    // has to leave evidence. Only under the flag, so no default run's stderr
    // changes.
    if args.batched {
        print_batched_summary(
            machine.step_profile(),
            machine.config.peripheral_tick_interval,
        );
    }

    export_bus_trace_if_requested(&args.bus_trace_out, &machine.bus);
    export_display_if_requested(&args.display_out, &machine.bus);
    ExitCode::from(EXIT_PASS)
}

/// Fast-boot an ESP32-classic (LX6) ELF and run the step loop.
///
/// Mirrors the pattern in `crates/core/tests/e2e_esp32_epaper.rs`:
/// `configure_xtensa_esp32` + ELF load + set_pc(entry) + set_sp + step loop.
/// UART0 (0x3FF4_0000, STM32F1 layout, echo_stdout=true) carries the TIER1
/// protocol lines to the tier1 harness via stdout.
pub(crate) fn run_firmware_esp32(args: &RunArgs) -> ExitCode {
    use labwired_core::bus::SystemBus;
    use labwired_core::system::xtensa::configure_xtensa_esp32;
    use labwired_core::SimulationError;

    // Read the firmware ELF.
    let elf_bytes = match std::fs::read(&args.firmware) {
        Ok(b) => b,
        Err(e) => {
            eprintln!(
                "error: cannot read firmware ELF at {:?}: {e}",
                args.firmware
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let image = match labwired_loader::load_elf_bytes(&elf_bytes) {
        Ok(img) => img,
        Err(e) => {
            eprintln!("error: failed to parse ELF: {e}");
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let mut bus = SystemBus::new();
    let mut cpu = configure_xtensa_esp32(&mut bus);

    // Load ELF segments into bus memory (IRAM/DRAM/flash windows).
    for segment in &image.segments {
        for (i, &byte) in segment.data.iter().enumerate() {
            let addr = segment.start_addr + i as u64;
            let _ = bus.write_u8(addr, byte);
        }
    }

    // Set PC to ELF entry and seed SP at top of SRAM1 (post-BROM default on
    // real silicon; see e2e_external_arduino_esp32_in_sim for the rationale).
    // CHEAT(SKIP): bypasses the boot ROM and hand-seeds PC/SP. See FIDELITY.md §C.
    cpu.set_pc(image.entry_point as u32);
    cpu.set_sp(0x3FFE_0000);
    // Post-bootloader PS state: WOE=1 (windowed ABI), INTLEVEL=0, EXCM=0.
    cpu.ps = labwired_core::cpu::xtensa_regs::Ps::from_raw(1 << 18);

    let limit = args.max_steps.unwrap_or(u64::MAX);
    let mut steps = 0u64;

    // Drive the authoritative `Machine` lifecycle, not a hand-rolled
    // `cpu.step` + `bus.tick_peripherals_*` pair. See the note on
    // `run_firmware`'s loop: the hand-rolled shape never published the bus
    // cycle clock, which freezes every `uses_scheduler()` peripheral under
    // `--features event-scheduler`.
    let mut machine = labwired_core::Machine::new(cpu, bus);

    while steps < limit {
        match machine.step() {
            Ok(()) => {}
            Err(SimulationError::BreakpointHit(_)) => break,
            Err(SimulationError::ExceptionRaised { cause, pc }) => {
                eprintln!("labwired-cli run (esp32): ExceptionRaised cause={cause} at 0x{pc:08x}");
                return ExitCode::from(EXIT_RUNTIME_ERROR);
            }
            Err(e) => {
                eprintln!(
                    "labwired-cli run (esp32): simulator error at pc=0x{:08x}: {e}",
                    machine.cpu.get_pc(),
                );
                return ExitCode::from(EXIT_RUNTIME_ERROR);
            }
        }
        steps += 1;
    }
    eprintln!(
        "labwired-cli run (esp32): reached --max-steps {limit}; pc=0x{:08x}",
        machine.cpu.get_pc(),
    );
    export_bus_trace_if_requested(&args.bus_trace_out, &machine.bus);
    export_display_if_requested(&args.display_out, &machine.bus);
    ExitCode::from(EXIT_PASS)
}

pub(crate) fn run_firmware(
    args: RunArgs,
    plugins: &[&dyn labwired_core::plugin::ChipPlugin],
) -> ExitCode {
    use labwired_core::boot::esp32s3::{fast_boot, BootOpts};
    use labwired_core::bus::SystemBus;
    use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3BootMode, Esp32s3Opts};
    use labwired_core::SimulationError;

    // Read the chip YAML to validate the chip family.
    let chip_yaml = match std::fs::read_to_string(&args.chip) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read chip YAML at {:?}: {e}", args.chip);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    // ARM fast-boot path: parse the chip YAML, build the bus, run the firmware
    // through a Cortex-M machine, and stream UART bytes to stdout so the
    // TIER1 protocol lines are visible to the caller.
    if chip_yaml.contains("arch: \"arm\"") || chip_yaml.contains("arch: arm") {
        return run_firmware_arm(&args, &chip_yaml, plugins);
    }

    // RISC-V fast-boot path: load peripherals from the chip YAML and run the
    // RV32I core. This is the path used by Tier-1 fixtures for RISC-V chips
    // (e.g. ESP32-C3) which cannot go through the Xtensa boot sequence.
    if chip_yaml.contains("arch: \"riscv\"") || chip_yaml.contains("arch: riscv") {
        return run_firmware_riscv(args, chip_yaml, plugins);
    }

    // Everything below here is Xtensa, which `labwired run` drives with a raw
    // `cpu.step()` + `tick_peripherals_with_costs()` loop rather than through
    // `Machine` — there is no batched orchestration to select. Refuse rather
    // than accept the flag and run the unbatched loop anyway: a caller that
    // asked for the batched path and was quietly given the other one would
    // record a number for a path it never executed.
    if args.batched {
        eprintln!(
            "error: --batched is not available for chip {:?}: the Xtensa path \
             does not run through `Machine::advance`",
            args.chip,
        );
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }

    // Classic ESP32 (Xtensa LX6) fast-boot path.
    if chip_yaml.contains("xtensa-lx6") {
        return run_firmware_esp32(&args);
    }

    if !chip_yaml.contains("xtensa-lx7") {
        eprintln!(
            "error: chip {:?} does not look like an Xtensa LX7 chip; \
             only ESP32-S3 is supported by `labwired run`",
            args.chip,
        );
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }

    // Read the firmware ELF.
    let elf_bytes = match std::fs::read(&args.firmware) {
        Ok(b) => b,
        Err(e) => {
            eprintln!(
                "error: cannot read firmware ELF at {:?}: {e}",
                args.firmware
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    // Wire the bus + CPU.
    let mut bus = SystemBus::new();
    // `--rom-boot` runs the real ROM from reset, which programs the flash MMU;
    // select the MMU XIP model for it. Fast-boot uses identity per-window XIP.
    let opts = Esp32s3Opts {
        real_reset_boot: args.rom_boot,
        // Size the flash backing from the chip descriptor, not the 4 MiB
        // default: an N16R8 image puts data partitions well past 4 MiB (a WAD
        // at 0x410000, say), and a short backing truncates them to 0xFF with no
        // error — the partition table still reads fine, so it looks like a
        // corrupt asset rather than a too-small model.
        flash_size: labwired_config::ChipDescriptor::from_file(&args.chip)
            .map(|c| c.flash.size as u32)
            .unwrap_or(Esp32s3Opts::default().flash_size),
        ..Esp32s3Opts::default()
    };
    let wiring = configure_xtensa_esp32s3(&mut bus, &opts);
    let boot_mode = wiring.boot_mode; // Copy before cpu is moved out of wiring

    // Install default tracing GPIO observer.
    wiring.add_gpio_observer(
        &mut bus,
        std::sync::Arc::new(crate::gpio_observer::TracingGpioObserver::new()),
    );

    // Optional JSON-line GPIO trace.
    if let Some(path) = &args.gpio_trace {
        match crate::gpio_observer::JsonGpioObserver::new(path) {
            Ok(obs) => {
                wiring.add_gpio_observer(&mut bus, std::sync::Arc::new(obs));
                eprintln!("labwired-cli run: gpio trace -> {:?}", path);
            }
            Err(e) => {
                eprintln!("error: cannot open gpio-trace file {:?}: {e}", path);
                return ExitCode::from(EXIT_CONFIG_ERROR);
            }
        }
    }

    // Manifest-declared board devices (`--system`). `configure_xtensa_esp32s3`
    // builds the peripheral bank in Rust and never runs `from_config`'s
    // peripheral loop, so the external devices are attached here through the
    // same canonical resolver every other family uses.
    if let Some(sys_path) = &args.system {
        let manifest = match labwired_config::SystemManifest::from_file(sys_path) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("error: cannot read system manifest {sys_path:?}: {e}");
                return ExitCode::from(EXIT_CONFIG_ERROR);
            }
        };
        if let Err(e) =
            labwired_core::system::xtensa::attach_esp32_external_devices(&mut bus, &manifest)
        {
            eprintln!("error: cannot attach external devices from {sys_path:?}: {e:#}");
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
        eprintln!(
            "labwired-cli run: attached {} external device(s) from {:?}",
            manifest.external_devices.len(),
            sys_path
        );
    }

    // `--stop-on`: mirror the USB-Serial-JTAG console into a buffer the run
    // loop can search. `echo_stdout` stays true so the transcript still
    // streams to stdout exactly as without the flag.
    let stop_sink: Option<std::sync::Arc<std::sync::Mutex<Vec<u8>>>> =
        args.stop_on.as_ref().and_then(|_| {
            use labwired_core::peripherals::esp32s3::usb_serial_jtag::UsbSerialJtag;
            let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            for p in &mut bus.peripherals {
                if p.name != labwired_core::console::USB_SERIAL_JTAG {
                    continue;
                }
                if let Some(j) = p
                    .dev
                    .as_any_mut()
                    .and_then(|a| a.downcast_mut::<UsbSerialJtag>())
                {
                    j.set_sink(Some(sink.clone()), true);
                    return Some(sink);
                }
            }
            eprintln!("error: --stop-on needs the USB-Serial-JTAG console; none found");
            None
        });
    let mut stop_scanned: usize = 0;

    let mut cpu = wiring.cpu;

    // Dual-core (SMP): the APP_CPU (core 1), on BOTH boot paths — the same
    // shape `system::node::build_esp32s3_node` builds, so the single-chip
    // runner and the world runner share ONE bring-up mechanism.
    //
    //   * rom-boot: core 1 is created halted at the ROM reset vector and
    //     released when the PRO_CPU clears CORE_1_RESETING (the real hardware
    //     edge, surfaced by the SYSTEM_CORE_1_CONTROL peripheral as
    //     APPCPU_RESET_RELEASED). It then boots the real ROM like silicon.
    //   * fast-boot: there is no ROM to run, so core 1 is released by
    //     `Machine`'s boundary when the `ets_set_appcpu_boot_addr` ROM thunk
    //     hands over `call_start_cpu1` (APPCPU_BOOT_ADDR).
    //
    // Neither reads a firmware symbol. This replaces the handshake pre-paint
    // the fast-boot path used to do (writing 1 into `s_cpu_inited`,
    // `s_other_cpu_startup_done`, … resolved from the ELF) — a thunk that
    // faked the *result* of a core-1 boot that never happened.
    let mut cpu1 = Some(labwired_core::cpu::xtensa_lx7::XtensaLx7::new_app_cpu());
    let mut appcpu_started = false;

    if args.rom_boot {
        // ── Faithful boot: run the real ROM from the reset vector ──────────
        // The CPU resets to 0x40000400 (BROM reset vector). With the real ROM
        // (auto-provisioned, or pinned via LABWIRED_ESP32S3_ROM) and the flash image behind the SPI-flash
        // controller (LABWIRED_ESP32S3_FLASH), the chip's own boot ROM loads
        // the 2nd-stage bootloader + app and jumps to it — same path as
        // silicon. No fast_boot, no ELF pre-load, no handshake pre-paint.
        let _ = &elf_bytes; // ELF used only for symbol/diagnostic context
                            // --rom-boot runs the genuine boot ROM. The ROM is auto-provisioned from
                            // the installed toolchain by configure_xtensa_esp32s3 (or pinned via
                            // LABWIRED_ESP32S3_ROM/_DROM); we only need the flash image here. If no
                            // real ROM was resolved we are in harness mode, where --rom-boot is
                            // meaningless — fail clearly.
        if boot_mode != Esp32s3BootMode::Faithful {
            eprintln!(
                "error: --rom-boot needs the real ESP32-S3 boot ROM, but none was found. \
                 Install the ESP toolchain (PlatformIO/ESP-IDF) or set LABWIRED_ESP32S3_ROM_ELF \
                 (or pin LABWIRED_ESP32S3_ROM/_DROM)."
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
        if std::env::var("LABWIRED_ESP32S3_FLASH").is_err() {
            eprintln!(
                "error: --rom-boot needs LABWIRED_ESP32S3_FLASH set (the firmware flash image)"
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
        eprintln!(
            "labwired-cli run: ROM-boot from reset vector 0x{:08x} (real ROM + flash controller)",
            cpu.get_pc(),
        );
        // Faithful windowed-register machinery: rom-boot runs the real ROM +
        // firmware, which install the OF/UF window vectors and build a proper
        // stack save chain — so use the real per-access overflow / RETW
        // underflow path (no sim shadow stack).
        cpu.faithful_windows = true;
        if let Some(c1) = cpu1.as_mut() {
            c1.faithful_windows = true;
            eprintln!(
                "labwired-cli run: APP_CPU created (halted at reset vector 0x{:08x})",
                c1.get_pc(),
            );
        }
    } else {
        // Fast-boot.
        let boot = match fast_boot(
            &elf_bytes,
            &mut bus,
            &mut cpu,
            &BootOpts {
                stack_top_fallback: 0x3FCD_FFF0,
                icache_backing: Some(wiring.icache_backing),
                dcache_backing: Some(wiring.dcache_backing),
                factory_flash_base: None,
            },
        ) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("error: fast_boot failed: {e}");
                return ExitCode::from(EXIT_RUNTIME_ERROR);
            }
        };
        eprintln!(
            "labwired-cli run: entry=0x{:08x} stack=0x{:08x} segments={}",
            boot.entry, boot.stack, boot.segments_loaded,
        );
        // NOTE: no ESP-IDF dual-core handshake pre-paint here. The single-chip
        // runner used to resolve `s_cpu_inited` / `s_cpu_up` / `s_system_inited`
        // / `s_resume_cores` / `s_other_cpu_startup_done` out of the ELF and
        // write 1 into each, because it ran ONE cpu and core 1 could never mark
        // them itself. Core 1 is real on both paths now (see the `cpu1`
        // construction above), so those flags are set by the firmware running on
        // core 1 — including `s_other_cpu_startup_done`, which core 1's FreeRTOS
        // idle hook writes only after its systimer tick wakes it out of WAITI.
    }

    // Run the step loop through the authoritative `Machine` lifecycle.
    //
    // This loop used to be a hand-rolled `cpu.step()` + `cpu1.step()` +
    // `bus.tick_peripherals_with_costs()` triple. That reproduces only the
    // legacy-walk half of the lifecycle and silently drops the other half:
    // it never publishes the bus cycle clock (`SystemBus::set_current_cycle`)
    // and it has no event scheduler at all, because the scheduler heap lives
    // on `Machine`. Under `--features event-scheduler` the per-cycle walk
    // SKIPS every `uses_scheduler()` peripheral (`bus/tick.rs`, the
    // `p.dev.uses_scheduler()` early-return in the walk), so those two
    // mechanisms are the ONLY things that advance them — leaving TIMG frozen
    // at cycle 0 and the UART TX FIFO undrained, which spins the S3 boot ROM
    // forever inside `uart_tx_one_char_uart`. `Machine::step`'s own doc says
    // frontends "must not reproduce the lifecycle with direct `Cpu::step`
    // calls"; this is why.
    //
    // `Machine::step` is `advance(AdvanceRequest::single())`: one primary
    // quantum, then the secondary, then one peripheral boundary — the same
    // order and the same cadence the hand-rolled loop had.
    let limit = args.max_steps.unwrap_or(u64::MAX);
    let mut machine = match cpu1 {
        Some(c1) => labwired_core::Machine::new(cpu, bus).with_secondary_cpu(c1),
        None => labwired_core::Machine::new(cpu, bus),
    };
    let mut steps = 0u64;
    // Ring buffer of recent PCs for post-mortem on exceptions.
    const RING_LEN: usize = 1024;
    let mut pc_ring: [u32; RING_LEN] = [0; RING_LEN];
    let mut ring_head: usize = 0;
    let smp_trace = std::env::var("LABWIRED_SMP_TRACE").is_ok();
    let dense_from: u64 = std::env::var("LABWIRED_DENSE_FROM")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(u64::MAX);
    let dense_len: u64 = std::env::var("LABWIRED_DENSE_LEN")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(800);
    // First-hit watchpoints for the SMP startup → first-task-dispatch path
    // (addresses from firmware.elf for this Unity demo). Each tracks whether
    // it's been reported on core 0 / core 1 yet.
    let mut watch: [(u32, &str, [bool; 2]); 11] = [
        (0x4037ec3c, "xPortStartScheduler", [false; 2]),
        (0x4037f064, "_frxt_dispatch", [false; 2]),
        (0x4037f067, "dispatch:post-switchctx", [false; 2]),
        (0x4037f08f, "dispatch:retw-into-task", [false; 2]),
        (0x4037fd64, "vTaskSwitchContext", [false; 2]),
        (0x4037f960, "prvIdleTask", [false; 2]),
        (0x4202240c, "esp_startup_start_app", [false; 2]),
        (0x4202239c, "main_task", [false; 2]),
        (0x420047c0, "app_main", [false; 2]),
        (0x42002040, "setup()", [false; 2]),
        (0x42001f90, "UnityBegin", [false; 2]),
    ];
    // Debug breakpoints / memory watches (parse hex; ignore unparseable).
    let parse_hex = |s: &str| -> Option<u32> {
        u32::from_str_radix(s.trim_start_matches("0x").trim_start_matches("0X"), 16).ok()
    };
    let break_at: Vec<u32> = args.break_at.iter().filter_map(|s| parse_hex(s)).collect();
    let watch_mem: Vec<u32> = args.watch_mem.iter().filter_map(|s| parse_hex(s)).collect();
    let mut break_hit = vec![false; break_at.len()]; // PRO_CPU first-hit flags
    let mut break_hit1 = vec![false; break_at.len()]; // APP_CPU first-hit flags
                                                      // On the first time a core's PC reaches a --break-at address, dump its
                                                      // a0..a15 + window state and the --watch-mem words. Covers both cores so an
                                                      // APP_CPU fault is observable too.
    macro_rules! check_break {
        ($c:expr, $pc:expr, $hits:expr) => {
            if let Some(bi) = break_at.iter().position(|&b| b == $pc) {
                if !$hits[bi] {
                    $hits[bi] = true;
                    eprintln!(
                        "labwired-cli run: BREAK-AT 0x{:08x} (step {steps}, core {})",
                        $pc,
                        if $c.app_cpu { 1 } else { 0 }
                    );
                    for r in 0..16u8 {
                        eprintln!("    a{:<2} = 0x{:08x}", r, $c.regs.read_logical(r));
                    }
                    eprintln!(
                        "    PS=0x{:08x} WB={} WS=0x{:04x}",
                        $c.ps.as_raw(),
                        $c.regs.windowbase(),
                        $c.regs.windowstart()
                    );
                    for &m in &watch_mem {
                        match machine.bus.read_u32(m as u64) {
                            Ok(v) => eprintln!("    mem[0x{m:08x}] = 0x{v:08x}"),
                            Err(e) => eprintln!("    mem[0x{m:08x}] = <unmapped: {e}>"),
                        }
                    }
                }
            }
        };
    }
    if !break_at.is_empty() {
        eprintln!(
            "labwired-cli run: breakpoints {:?} watch-mem {:?}",
            break_at
                .iter()
                .map(|a| format!("0x{a:08x}"))
                .collect::<Vec<_>>(),
            watch_mem
                .iter()
                .map(|a| format!("0x{a:08x}"))
                .collect::<Vec<_>>(),
        );
    }

    while steps < limit {
        let pc_before = machine.cpu.get_pc();
        pc_ring[ring_head] = pc_before;
        ring_head = (ring_head + 1) % RING_LEN;

        // Debug breakpoint (PRO_CPU): dump on first hit.
        check_break!(machine.cpu, pc_before, break_hit);

        // Debug breakpoint (APP_CPU): dump on first hit, before the machine
        // steps it. `Machine::step` drives both cores inside one call, so the
        // APP_CPU's pre-step PC has to be sampled here.
        if let Some(pc1) = machine.cpu_secondary.as_ref().map(|c| c.get_pc()) {
            check_break!(machine.cpu_secondary.as_ref().unwrap(), pc1, break_hit1);
        }

        // Capture the APP_CPU entry when PRO_CPU programs it. The ROM also
        // points the APP_CPU at early DRAM stubs during its own bring-up; only
        // a real code entry (app IRAM/XIP, >= 0x4037_0000 — excludes ROM and
        // DRAM) is the application's `call_start_cpu1`.
        // Release the APP_CPU on the real hardware edge: the PRO_CPU clearing
        // CORE_1_RESETING (signalled by the SYSTEM_CORE_1_CONTROL peripheral).
        // The APP_CPU then boots the real ROM from its reset vector — exactly
        // like silicon, no firmware-symbol hooks.
        if !appcpu_started
            && labwired_core::peripherals::esp_xtensa_common::rom_thunks::APPCPU_RESET_RELEASED
                .with(|s| s.take())
        {
            appcpu_started = true;
            if let Some(c1) = machine.cpu_secondary.as_mut() {
                c1.halted = false;
            }
            eprintln!(
                "labwired-cli run: APP_CPU released from reset → booting real ROM (step {steps})"
            );
        }

        match machine.step() {
            Ok(()) => {}
            Err(SimulationError::BreakpointHit(pc)) => {
                eprintln!("labwired-cli run: BREAK at 0x{pc:08x}");
                export_bus_trace_if_requested(&args.bus_trace_out, &machine.bus);
                export_display_if_requested(&args.display_out, &machine.bus);
                return ExitCode::from(EXIT_PASS);
            }
            Err(SimulationError::ExceptionRaised { cause, pc }) => {
                eprintln!("labwired-cli run: ExceptionRaised cause={cause} at 0x{pc:08x}");
                eprintln!(
                    "labwired-cli run: PS=0x{:08x} (excm={} intlevel={}) WB={} WS=0x{:04x}",
                    machine.cpu.ps.as_raw(),
                    machine.cpu.ps.excm(),
                    machine.cpu.ps.intlevel(),
                    machine.cpu.regs.windowbase(),
                    machine.cpu.regs.windowstart(),
                );
                eprintln!("labwired-cli run: recent PCs (oldest first):");
                for i in 0..RING_LEN {
                    let idx = (ring_head + i) % RING_LEN;
                    if pc_ring[idx] != 0 {
                        eprintln!("  [{:2}] 0x{:08x}", i, pc_ring[idx]);
                    }
                }
                return ExitCode::from(EXIT_RUNTIME_ERROR);
            }
            Err(e) => {
                eprintln!(
                    "labwired-cli run: simulator error at pc=0x{:08x}: {e}",
                    machine.cpu.get_pc(),
                );
                eprintln!("labwired-cli run: a0..a15 at fault:");
                for r in 0..16u8 {
                    eprintln!("  a{:<2} = 0x{:08x}", r, machine.cpu.regs.read_logical(r));
                }
                eprintln!(
                    "  WB=0x{:x} WS=0x{:04x}",
                    machine.cpu.regs.windowbase(),
                    machine.cpu.regs.windowstart(),
                );
                eprintln!("labwired-cli run: recent PCs (oldest first):");
                for i in 0..RING_LEN {
                    let idx = (ring_head + i) % RING_LEN;
                    if pc_ring[idx] != 0 {
                        eprintln!("  [{:2}] 0x{:08x}", i, pc_ring[idx]);
                    }
                }
                return ExitCode::from(EXIT_RUNTIME_ERROR);
            }
        }
        // panic_abort(details) reason printer (gated): the ESP-IDF panic path
        // stores the assert/abort string ptr in a2 just before the trap. Helps
        // pinpoint firmware-level aborts during bring-up.
        if std::env::var("LABWIRED_CCDBG").is_ok() {
            // Collect the string pointers first: reading them back needs
            // `&mut machine.bus`, so the core borrows have to be released.
            let panic_args: Vec<u32> = [Some(&machine.cpu), machine.cpu_secondary.as_ref()]
                .into_iter()
                .flatten()
                .filter(|c| c.get_pc() == 0x4037_e0a3)
                .map(|c| c.regs.read_logical(2))
                .collect();
            for p in panic_args {
                let mut s = String::new();
                for i in 0..160u32 {
                    match machine.bus.read_u8(p as u64 + i as u64) {
                        Ok(0) | Err(_) => break,
                        Ok(b) => s.push(b as char),
                    }
                }
                eprintln!("CCDBG: panic \"{s}\" step={steps}");
            }
        }
        steps += 1;

        // `--stop-on <text>`: end the run as soon as the firmware's console
        // says so. Makes end-of-run artifacts (`--display-out`) frame-exact —
        // "stop right after the firmware printed its own frame thumbnail" is
        // reproducible, a hand-tuned `--max-steps` is not. Scanned in slices
        // from a cursor so a long run does not re-read the whole transcript.
        if let (Some(sink), Some(pat)) = (&stop_sink, &args.stop_on) {
            if steps.is_multiple_of(100_000) {
                let buf = sink.lock().unwrap();
                if buf.len() > stop_scanned {
                    let from = stop_scanned.saturating_sub(pat.len());
                    if String::from_utf8_lossy(&buf[from..]).contains(pat.as_str()) {
                        drop(buf);
                        eprintln!("labwired-cli run: --stop-on {pat:?} matched at step {steps}");
                        break;
                    }
                    stop_scanned = buf.len();
                }
            }
        }

        // SMP bring-up tracer (gated). Prints both cores' PCs periodically and
        // flags the first time each core enters app XIP code (>= 0x4200_0000,
        // where setup()/loop()/Unity live) — the signal that the FreeRTOS SMP
        // scheduler finally dispatched the pinned loopTask.
        if smp_trace {
            let app_pc = machine
                .cpu_secondary
                .as_ref()
                .map(|c| c.get_pc())
                .unwrap_or(0);
            for (core, pc) in [(0usize, machine.cpu.get_pc()), (1usize, app_pc)] {
                for w in watch.iter_mut() {
                    if w.0 == pc && !w.2[core] {
                        w.2[core] = true;
                        eprintln!("SMP: core {core} reached {} (0x{pc:08x}) step {steps}", w.1);
                    }
                }
            }
            if steps.is_multiple_of(10_000_000) {
                eprintln!(
                    "SMP: step {steps:>11}  pro=0x{:08x}  app=0x{app_pc:08x}",
                    machine.cpu.get_pc(),
                );
            }
            // Dense single-step trace window (env LABWIRED_DENSE_FROM / _LEN)
            // for following a context switch instruction-by-instruction.
            if steps >= dense_from && steps < dense_from + dense_len {
                eprintln!(
                    "D {steps} pro=0x{:08x} ps={:x} wb={} ws=0x{:04x} exc={} epc1=0x{:08x} | app=0x{app_pc:08x}",
                    machine.cpu.get_pc(),
                    machine.cpu.ps.as_raw(),
                    machine.cpu.regs.windowbase(),
                    machine.cpu.regs.windowstart(),
                    machine.cpu.sr.read(232),
                    machine.cpu.sr.read(177),
                );
            }
        }
    }
    // Optional end-of-run dump of the Unity result struct (env
    // LABWIRED_UNITY_ADDR=<hex base of the `Unity` UNITY_STORAGE_T global>).
    // Mirrors the hardware oracle (`mdw <addr> 10`): NumberOfTests at +20,
    // TestFailures at +24, TestIgnores at +28 — the authoritative pass/fail
    // since Unity's text output goes out USB_SERIAL_JTAG, not stdout.
    if let Ok(s) = std::env::var("LABWIRED_UNITY_ADDR") {
        if let Ok(base) = u32::from_str_radix(s.trim_start_matches("0x"), 16) {
            let mut words = [0u32; 10];
            for (i, w) in words.iter_mut().enumerate() {
                *w = machine
                    .bus
                    .read_u32(base as u64 + (i * 4) as u64)
                    .unwrap_or(0);
            }
            eprint!("labwired-cli run: Unity@0x{base:08x}:");
            for w in &words {
                eprint!(" {w:08x}");
            }
            eprintln!();
            eprintln!(
                "labwired-cli run: Unity NumberOfTests={} TestFailures={} TestIgnores={}",
                words[5], words[6], words[7],
            );
        }
    }
    let cpu1_pc = machine
        .cpu_secondary
        .as_ref()
        .map(|c| format!(" appcpu_pc=0x{:08x}", c.get_pc()))
        .unwrap_or_default();
    eprintln!(
        "labwired-cli run: reached --max-steps {limit}; pc=0x{:08x}{cpu1_pc}",
        machine.cpu.get_pc(),
    );
    export_bus_trace_if_requested(&args.bus_trace_out, &machine.bus);
    export_display_if_requested(&args.display_out, &machine.bus);
    ExitCode::from(EXIT_PASS)
}

pub(crate) fn run_interactive(
    cli: Cli,
    plugins: &[&dyn labwired_core::plugin::ChipPlugin],
) -> ExitCode {
    info!("Starting LabWired Simulator");

    let Some(firmware) = &cli.firmware else {
        emit_error(
            cli.json,
            "ConfigError",
            "Missing required --firmware argument".to_string(),
            None,
            EXIT_CONFIG_ERROR,
        );
        return ExitCode::from(EXIT_CONFIG_ERROR);
    };

    let system_path = cli.system.clone();
    let resolved_system = match system_path
        .as_deref()
        .map(labwired_config::ResolvedSystem::from_manifest_file)
        .transpose()
    {
        Ok(s) => s,
        Err(e) => {
            emit_error(
                cli.json,
                "ConfigError",
                format!("Failed to load system manifest: {e:#}"),
                None,
                EXIT_CONFIG_ERROR,
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };
    let bus = match labwired_core::system::builder::build_system_bus_with_plugins(
        resolved_system.as_ref(),
        plugins,
    ) {
        Ok(bus) => bus,
        Err(e) => {
            emit_error(
                cli.json,
                "ConfigError",
                format!("{:#}", e),
                Some(serde_json::json!({
                    "system_path": system_path.as_ref().map(|p| p.display().to_string()),
                })),
                EXIT_CONFIG_ERROR,
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    info!("Loading firmware: {:?}", firmware);
    let program = match labwired_loader::load_elf(firmware) {
        Ok(program) => program,
        Err(e) => {
            emit_error(
                cli.json,
                "LoadError",
                format!("{:#}", e),
                Some(serde_json::json!({
                    "firmware_path": firmware.display().to_string(),
                })),
                EXIT_CONFIG_ERROR,
            );
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    info!("Firmware Loaded Successfully!");
    info!("Entry Point: {:#x}", program.entry_point);

    let metrics = std::sync::Arc::new(labwired_core::metrics::PerformanceMetrics::new());

    let cpu_arch = if let Some(sys_path) = &system_path {
        match labwired_config::SystemManifest::from_file(sys_path) {
            Ok(manifest) => {
                let chip_dir = sys_path.parent().unwrap_or_else(|| Path::new("."));
                match labwired_config::ChipDescriptor::resolve_with(
                    &manifest.chip,
                    chip_dir,
                    &crate::plugin_chip_yaml(plugins),
                ) {
                    Ok(c) => c.arch,
                    Err(e) => {
                        emit_error(
                            cli.json,
                            "ConfigError",
                            format!("Failed to parse chip descriptor: {:#}", e),
                            Some(serde_json::json!({
                                "chip": manifest.chip.clone(),
                            })),
                            EXIT_CONFIG_ERROR,
                        );
                        return ExitCode::from(EXIT_CONFIG_ERROR);
                    }
                }
            }
            Err(e) => {
                emit_error(
                    cli.json,
                    "ConfigError",
                    format!("Failed to parse system manifest: {:#}", e),
                    Some(serde_json::json!({
                        "system_path": sys_path.display().to_string(),
                    })),
                    EXIT_CONFIG_ERROR,
                );
                return ExitCode::from(EXIT_CONFIG_ERROR);
            }
        }
    } else {
        // Default to Arm if no system config provided (backward compatibility)
        labwired_config::Arch::Arm
    };

    if program.arch != labwired_core::Arch::Unknown {
        // Map core::Arch to config::Arch for comparison
        let prog_arch = match program.arch {
            labwired_core::Arch::Arm => labwired_config::Arch::Arm,
            labwired_core::Arch::RiscV => labwired_config::Arch::RiscV,
            labwired_core::Arch::XtensaLx7 => labwired_config::Arch::Xtensa,
            labwired_core::Arch::Avr => labwired_config::Arch::Avr,
            _ => labwired_config::Arch::Unknown,
        };

        if prog_arch != cpu_arch {
            tracing::warn!(
                "Architecture Mismatch! Config expects {:?}, but ELF is {:?}",
                cpu_arch,
                prog_arch
            );
        }
    }

    match cpu_arch {
        labwired_config::Arch::Arm => run_interactive_arm(cli, bus, program, metrics),
        labwired_config::Arch::RiscV => run_interactive_riscv(cli, bus, program, metrics),
        labwired_config::Arch::Xtensa => run_interactive_xtensa(cli, bus, program, metrics),
        _ => {
            emit_error(
                cli.json,
                "ConfigError",
                format!("Unsupported architecture: {:?}", cpu_arch),
                Some(serde_json::json!({
                    "architecture": format!("{:?}", cpu_arch),
                })),
                EXIT_CONFIG_ERROR,
            );
            ExitCode::from(EXIT_CONFIG_ERROR)
        }
    }
}

/// Fast-boot an ARM Cortex-M firmware from a chip YAML and ELF path.
///
/// Builds the bus directly from the chip descriptor (no system manifest
/// required — the chip YAML's `peripherals` list is sufficient for raw-register
/// fixture firmware).  UART bytes are streamed to stdout so the TIER1 protocol
/// lines are visible to callers that pipe stdout.  Exits when the step limit
/// is reached or the firmware halts.
pub(crate) fn run_firmware_arm(
    args: &RunArgs,
    chip_yaml: &str,
    plugins: &[&dyn labwired_core::plugin::ChipPlugin],
) -> ExitCode {
    use labwired_config::{ChipDescriptor, SystemManifest};
    use labwired_core::bus::SystemBus;
    use labwired_core::system::cortex_m::configure_cortex_m;
    use labwired_core::Machine;
    use std::io::Write;

    // Parse the chip descriptor.
    let chip = match serde_yaml::from_str::<ChipDescriptor>(chip_yaml) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: cannot parse chip YAML: {e}");
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    // Synthesise a minimal system manifest (no external devices) so the bus
    // builder has something to work with.  The chip path is already absolute
    // because `chip_yaml` was read from `args.chip`.
    let manifest_yaml = format!(
        "name: \"tier1-run\"\nchip: \"{}\"\nexternal_devices: []\n",
        args.chip.display()
    );
    let mut manifest = match serde_yaml::from_str::<SystemManifest>(&manifest_yaml) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: cannot build minimal manifest: {e}");
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };
    // Chip field must be an absolute path string; already is (args.chip is absolute
    // relative to the caller's cwd, which is the workspace root per run_target).
    manifest.chip = args.chip.to_string_lossy().into_owned();

    // Build the bus.
    let mut bus = match SystemBus::from_config_with_plugins(&chip, &manifest, plugins) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: cannot build bus from chip config: {e}");
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    // Attach stdout echo to every UART so protocol lines flow through.
    // `echo_stdout = true` prints each byte as it arrives.
    let uart_sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    bus.attach_uart_tx_sink(uart_sink.clone(), true);

    // Configure Cortex-M CPU.
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);

    // Load ELF.
    let mut image = match labwired_loader::load_elf(&args.firmware) {
        Ok(img) => img,
        Err(e) => {
            eprintln!("error: cannot load firmware ELF {:?}: {e}", args.firmware);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    // Multi-image flash composition (`--flash-image <path>@<hex-offset>`,
    // repeatable): additional pieces (SoftDevice, bootloader, ...) placed at
    // explicit absolute addresses alongside `--firmware`. Only touched when
    // at least one `--flash-image` is given, so the single-image path above
    // is completely unaffected when it is not.
    if !args.flash_image.is_empty() {
        use labwired_loader::multi_image::{
            check_no_overlaps, elf_alloc_sections, load_flash_piece, parse_flash_image_arg,
        };

        // Re-derive the primary --firmware's own segments from its ELF
        // section headers (SHF_ALLOC sections with real bytes) rather than
        // the PT_LOAD-based `image.segments` from `load_elf` above: some
        // toolchains (e.g. Adafruit's nRF52 core, whose linker scripts
        // request 64KB PT_LOAD alignment for DFU) emit a PT_LOAD segment
        // whose p_paddr is rounded down below the real code, backed on disk
        // by nothing but ELF-header bytes and zero padding — loading that
        // via p_paddr would plant that padding over a legitimately-owned
        // range (e.g. the SoftDevice below the app). See
        // `elf_alloc_sections` for the full rationale. Only applies when
        // `--flash-image` is in play; the single-image path above is
        // unaffected.
        let firmware_bytes = match std::fs::read(&args.firmware) {
            Ok(b) => b,
            Err(e) => {
                eprintln!(
                    "error: cannot re-read firmware ELF {:?}: {e}",
                    args.firmware
                );
                return ExitCode::from(EXIT_CONFIG_ERROR);
            }
        };
        image.segments = match elf_alloc_sections(&firmware_bytes) {
            Ok(secs) => secs
                .into_iter()
                .map(|(addr, data)| labwired_core::memory::Segment {
                    start_addr: addr,
                    data,
                })
                .collect(),
            Err(e) => {
                eprintln!(
                    "error: cannot extract ALLOC sections from firmware ELF {:?}: {e:#}",
                    args.firmware
                );
                return ExitCode::from(EXIT_CONFIG_ERROR);
            }
        };

        for arg in &args.flash_image {
            let (path, offset) = match parse_flash_image_arg(arg) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("error: invalid --flash-image {arg:?}: {e:#}");
                    return ExitCode::from(EXIT_CONFIG_ERROR);
                }
            };
            let piece = match load_flash_piece(&path, offset) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("error: cannot load --flash-image {arg:?}: {e:#}");
                    return ExitCode::from(EXIT_CONFIG_ERROR);
                }
            };
            image.segments.extend(piece.segments);
        }

        if let Err(e) = check_no_overlaps(&image.segments) {
            eprintln!("error: --flash-image composition failed: {e:#}");
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    }

    if let Err(e) = machine.load_firmware(&image) {
        eprintln!("error: cannot map firmware into bus: {e}");
        return ExitCode::from(EXIT_RUNTIME_ERROR);
    }

    // Run the step loop.
    let limit = args.max_steps.unwrap_or(u64::MAX);

    // Opt-in batched orchestration (--batched): the path the browser runs.
    // Kept behind the flag so the default `labwired run` for ARM — TIER1
    // fixtures, labs, every existing test — keeps the exact `machine.step()`
    // loop, byte for byte.
    //
    // Both loops live in their own `#[inline(never)]` function, and this is not
    // cosmetic: with the two inlined into one body, adding the batched arm cost
    // the SINGLE-STEP loop ~12 Ir/step (+0.6% on stm32l476) with its source
    // untouched — LLVM's register allocation over the merged function changed.
    // Splitting them puts each loop back in a frame whose codegen does not
    // depend on the other's existence, which is what "the default path is
    // unaffected" has to mean for a gate that measures instructions.
    let faulted = if args.batched {
        run_arm_batched_loop(&mut machine, limit)
    } else {
        run_arm_step_loop(&mut machine, limit)
    };

    // Flush stdout.
    let _ = std::io::stdout().flush();
    export_bus_trace_if_requested(&args.bus_trace_out, &machine.bus);
    export_display_if_requested(&args.display_out, &machine.bus);

    // A run that ended on a fault reports a fault. It used to print the error
    // and exit 0, so `labwired run … && echo ok` printed ok for firmware that
    // died on its second instruction, and any CI judging by exit status read a
    // memory access violation as a pass. Xtensa already exited non-zero here;
    // ARM is now consistent with it. `--allow-sim-error` restores the old
    // behaviour for callers that own the verdict themselves — the TIER1 matrix
    // reads protocol lines from stdout and ignores the exit code either way.
    if faulted && !args.allow_sim_error {
        return ExitCode::from(EXIT_RUNTIME_ERROR);
    }
    ExitCode::from(EXIT_PASS)
}

/// The ARM default: one simulated instruction per `Machine::step()` call.
#[inline(never)]
fn run_arm_step_loop(
    machine: &mut labwired_core::Machine<labwired_core::cpu::CortexM>,
    limit: u64,
) -> bool {
    let dbg_trace: u64 = std::env::var("LABWIRED_ARM_TRACE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    for i in 0..limit {
        if dbg_trace != 0 && i % dbg_trace == 0 {
            eprintln!("[arm-trace] step {i} pc={:#010x}", machine.cpu.get_pc());
        }
        // `advance` rather than `step`: `step` discards the AdvanceReport, and
        // a firmware-authored verdict appears nowhere else.
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
            Err(e) => {
                eprintln!("labwired run (arm): simulation error: {e}");
                // The caller decides what this means. It ends the run either
                // way; whether it ends the PROCESS with a failure is up to
                // `--allow-sim-error`, because a TIER1 protocol may already
                // have printed its verdict before the fault.
                return true;
            }
        }
    }
    false
}

/// One line of proof that the batched path ran, and how wide its batches were.
///
/// Printed only under `--batched`, so no default run's stderr changes. A caller
/// that asks for the batched path and gets no `[batched]` line back knows the
/// run did not take it — which is the difference between a measurement and a
/// guess. `steps_per_batch` is the observable that separates "batched" from
/// "batched in name only": at 1.00 the orchestration is issuing one instruction
/// per CPU dispatch and the batch window bought nothing.
fn print_batched_summary(profile: labwired_core::StepProfile, tick_interval: u32) {
    let per_batch = if profile.cpu_batches == 0 {
        0.0
    } else {
        profile.cpu_instructions as f64 / profile.cpu_batches as f64
    };
    eprintln!(
        "[batched] instructions={} batches={} steps_per_batch={:.2} \
         tick_interval={} peripheral_ticks={}",
        profile.cpu_instructions,
        profile.cpu_batches,
        per_batch,
        tick_interval,
        profile.peripheral_ticks,
    );
}

/// The ARM (Cortex-M) batched hot path: drive the run through
/// `Machine::advance(AdvanceRequest::run(..))` — the exact call the browser
/// makes from `Sim::step_batch` in `crates/wasm/src/lib.rs` — instead of the
/// `machine.step()` loop, which pins the CPU quantum to one instruction.
///
/// Why this exists at all: the wasm front end and the CLI had diverged on ARM.
/// Everything a user sees in the browser goes through `advance`, and #830
/// removed three clamps that had pinned that path to a one-instruction quantum
/// (9-16x native throughput on ARM boards) — yet the throughput gate drove ARM
/// through `machine.step()` and moved by 0.2-0.4%, because it never entered the
/// batched path. A regression in batch orchestration was invisible on the only
/// path users run.
///
/// The tick interval comes from `bus.max_safe_tick_interval()`, not from a
/// constant: that is the same source the browser reads through the wasm
/// `recommended_tick_interval` getter before calling
/// `set_peripheral_tick_interval`. A bus that reports 1 (anything non-relaxable
/// on it) therefore batches at 1 here too, exactly as it would in the browser —
/// which is a real property of that board, not a failure to engage, and the
/// `[batched]` line reports it as `steps_per_batch=1.00` rather than hiding it.
#[inline(never)]
fn run_arm_batched_loop(
    machine: &mut labwired_core::Machine<labwired_core::cpu::CortexM>,
    limit: u64,
) -> bool {
    use labwired_core::{AdvanceRequest, AdvanceStop};

    let mut faulted = false;

    let interval = machine.bus.max_safe_tick_interval();
    machine.config.peripheral_tick_interval = interval;
    machine.bus.config.peripheral_tick_interval = interval;

    // Chunk so an absent `--max-steps` (limit == u64::MAX) still bounds the fuel
    // handed to any single `advance` call, mirroring the RISC-V batched loop.
    // `advance` batches internally at the tick interval; the chunk only caps the
    // total instruction budget.
    const CHUNK: u64 = 4_000_000;
    let mut ran: u64 = 0;
    while ran < limit {
        let fuel = CHUNK.min(limit - ran);
        let before = machine.step_profile().cpu_instructions;
        let stop = match machine.advance(AdvanceRequest::run(Some(fuel))) {
            Ok(report) => Some(report.stop),
            Err(e) => {
                // Same contract as the single-step loop above.
                eprintln!("labwired run (arm, batched): simulation error: {e}");
                faulted = true;
                None
            }
        };
        let delta = machine.step_profile().cpu_instructions - before;
        ran += delta;
        match stop {
            // No forward progress (halt/idle with nothing left to skip): stop
            // rather than spin re-issuing empty batches up to `limit`.
            Some(AdvanceStop::NoProgress) | None => break,
            Some(_) if delta == 0 => break,
            Some(_) => {}
        }
    }

    print_batched_summary(machine.step_profile(), interval);
    faulted
}

pub(crate) fn run_interactive_arm(
    cli: Cli,
    mut bus: labwired_core::bus::SystemBus,
    program: labwired_core::memory::ProgramImage,
    metrics: Arc<labwired_core::metrics::PerformanceMetrics>,
) -> ExitCode {
    let (cpu, _nvic) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    let mut machine = labwired_core::Machine::new(cpu, bus);
    machine.observers.push(metrics.clone());

    if let Some(vcd_path) = &cli.vcd {
        let file = std::fs::File::create(vcd_path).expect("Failed to create VCD file");
        let observer = std::sync::Arc::new(vcd_trace::VcdObserver::new(file));
        machine.observers.push(observer);
    }

    if let Err(e) = machine.load_firmware(&program) {
        tracing::error!("Failed to load firmware into memory: {}", e);
        return ExitCode::from(EXIT_RUNTIME_ERROR);
    }

    info!("Starting Simulation (ARM Cortex-M)...");
    info!(
        "Initial PC: {:#x}, SP: {:#x}",
        machine.cpu.pc, machine.cpu.sp
    );

    // Check if GDB server is requested
    if let Some(port) = cli.gdb {
        let server = labwired_gdbstub::GdbServer::new(port);
        if let Err(e) = server.run(machine) {
            error!("GDB server failed: {}", e);
            return ExitCode::from(EXIT_RUNTIME_ERROR);
        }
        return ExitCode::from(EXIT_PASS);
    }

    let result = run_simulation_loop(&cli, &mut machine, &metrics);

    if let Some(path) = &cli.snapshot {
        // Need to reconstruct full paths or pass them?
        // cli.firmware is Option<PathBuf>, but checking run_interactive, it ensures firmware is set.
        // But run_interactive passed `program` not paths.
        // Creating cli passes ownership. `cli` has `firmware`.
        // `cli.system` is `Option<PathBuf>`.

        let firmware_path = cli.firmware.as_ref().expect("Firmware path required");
        let system_path = cli.system.as_ref();

        write_interactive_snapshot(
            path,
            &metrics,
            &machine,
            InteractiveSnapshotInputs {
                firmware_path,
                system_path,
                max_steps: cli.max_steps,
                steps_executed: result.steps_executed,
                stop_reason: result.stop_reason,
                message: result.stop_message,
            },
        );
    }

    crate::report::report_metrics(cli.json, &machine.cpu, &metrics);
    ExitCode::from(EXIT_PASS)
}

pub(crate) fn run_interactive_riscv(
    cli: Cli,
    mut bus: labwired_core::bus::SystemBus,
    program: labwired_core::memory::ProgramImage,
    metrics: Arc<labwired_core::metrics::PerformanceMetrics>,
) -> ExitCode {
    let cpu = labwired_core::system::riscv::configure_riscv(&mut bus);
    let mut machine = labwired_core::Machine::new(cpu, bus);
    machine.observers.push(metrics.clone());

    if let Some(vcd_path) = &cli.vcd {
        let file = std::fs::File::create(vcd_path).expect("Failed to create VCD file");
        let observer = std::sync::Arc::new(vcd_trace::VcdObserver::new(file));
        machine.observers.push(observer);
    }

    if let Err(e) = machine.load_firmware(&program) {
        tracing::error!("Failed to load firmware into memory: {}", e);
        return ExitCode::from(EXIT_RUNTIME_ERROR);
    }

    info!("Starting Simulation (RISC-V)...");
    info!(
        "Initial PC: {:#x}, SP: {:#x}",
        machine.cpu.pc,
        machine.cpu.x[2] // SP is x2 in RISC-V convention
    );

    // Check if GDB server is requested
    if let Some(port) = cli.gdb {
        let server = labwired_gdbstub::GdbServer::new(port);
        if let Err(e) = server.run(machine) {
            error!("GDB server failed: {}", e);
            return ExitCode::from(EXIT_RUNTIME_ERROR);
        }
        return ExitCode::from(EXIT_PASS);
    }

    let result = run_simulation_loop(&cli, &mut machine, &metrics);

    if let Some(path) = &cli.snapshot {
        let firmware_path = cli.firmware.as_ref().expect("Firmware path required");
        let system_path = cli.system.as_ref();

        write_interactive_snapshot(
            path,
            &metrics,
            &machine,
            InteractiveSnapshotInputs {
                firmware_path,
                system_path,
                max_steps: cli.max_steps,
                steps_executed: result.steps_executed,
                stop_reason: result.stop_reason,
                message: result.stop_message,
            },
        );
    }

    crate::report::report_metrics(cli.json, &machine.cpu, &metrics);
    ExitCode::from(EXIT_PASS)
}

pub(crate) fn run_interactive_xtensa(
    cli: Cli,
    mut bus: labwired_core::bus::SystemBus,
    program: labwired_core::memory::ProgramImage,
    metrics: Arc<labwired_core::metrics::PerformanceMetrics>,
) -> ExitCode {
    let cpu = labwired_core::system::xtensa::configure_xtensa(&mut bus);
    let mut machine = labwired_core::Machine::new(cpu, bus);
    machine.observers.push(metrics.clone());

    if let Some(vcd_path) = &cli.vcd {
        let file = std::fs::File::create(vcd_path).expect("Failed to create VCD file");
        let observer = std::sync::Arc::new(vcd_trace::VcdObserver::new(file));
        machine.observers.push(observer);
    }

    if let Err(e) = machine.load_firmware(&program) {
        tracing::error!("Failed to load firmware into memory: {}", e);
        return ExitCode::from(EXIT_RUNTIME_ERROR);
    }

    info!("Starting Simulation (Xtensa LX7)...");
    info!(
        "Initial PC: {:#x}, SP: {:#x}",
        machine.cpu.pc,
        machine.cpu.regs.read_logical(1) // SP is a1 in Xtensa
    );

    if cli.gdb.is_some() {
        error!("GDB server is not yet supported for Xtensa architecture");
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }

    let result = run_simulation_loop(&cli, &mut machine, &metrics);

    if let Some(path) = &cli.snapshot {
        let firmware_path = cli.firmware.as_ref().expect("Firmware path required");
        let system_path = cli.system.as_ref();

        write_interactive_snapshot(
            path,
            &metrics,
            &machine,
            InteractiveSnapshotInputs {
                firmware_path,
                system_path,
                max_steps: cli.max_steps,
                steps_executed: result.steps_executed,
                stop_reason: result.stop_reason,
                message: result.stop_message,
            },
        );
    }

    crate::report::report_metrics(cli.json, &machine.cpu, &metrics);
    ExitCode::from(EXIT_PASS)
}
