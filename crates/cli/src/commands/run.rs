// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `labwired run` + interactive (gdb/dap) drivers across ARM / RISC-V / Xtensa.

use crate::artifacts::{write_interactive_snapshot, InteractiveSnapshotInputs};
use crate::*;

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

pub(crate) fn run_firmware_riscv(args: RunArgs, _chip_yaml: String) -> ExitCode {
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
        schema_version: "1.0".to_string(),
        name: chip.name.clone(),
        chip: args.chip.to_string_lossy().into_owned(),
        memory_overrides: Default::default(),
        external_devices: vec![],
        board_io: vec![],
        debug_uart: None,
        peripherals: vec![],
        walk_deleted: Some(false),
    };

    // Two-station WiFi run (env LABWIRED_WIFI_DUAL): boot two C3 instances with
    // distinct MACs onto the shared VirtualWifi medium so they associate, get
    // distinct DHCP leases, and exchange traffic over one virtual AP.
    if args.rom_boot && std::env::var("LABWIRED_WIFI_DUAL").is_ok() {
        return run_two_c3_wifi(&args, &chip, &manifest);
    }

    let mut bus = match SystemBus::from_config(&chip, &manifest) {
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
        let sp_top =
            (chip.ram.base + labwired_config::parse_size(&chip.ram.size).unwrap_or(0)) as u32;
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
        if let Err(e) = machine.step() {
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

    export_bus_trace_if_requested(&args.bus_trace_out, &machine.bus);
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
    // so batching at RECOMMENDED_TICK_INTERVAL (64) is byte-identical to
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

    export_bus_trace_if_requested(&args.bus_trace_out, &machine.bus);
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
    let observers: Vec<std::sync::Arc<dyn labwired_core::SimulationObserver>> = Vec::new();
    let config = labwired_core::SimulationConfig::default();
    let mut steps = 0u64;

    while steps < limit {
        match cpu.step(&mut bus, &observers, &config) {
            Ok(()) => {}
            Err(SimulationError::BreakpointHit(_)) => break,
            Err(SimulationError::ExceptionRaised { cause, pc }) => {
                eprintln!("labwired-cli run (esp32): ExceptionRaised cause={cause} at 0x{pc:08x}");
                return ExitCode::from(EXIT_RUNTIME_ERROR);
            }
            Err(e) => {
                eprintln!(
                    "labwired-cli run (esp32): simulator error at pc=0x{:08x}: {e}",
                    cpu.get_pc(),
                );
                return ExitCode::from(EXIT_RUNTIME_ERROR);
            }
        }
        bus.tick_peripherals_with_costs();
        steps += 1;
    }
    eprintln!(
        "labwired-cli run (esp32): reached --max-steps {limit}; pc=0x{:08x}",
        cpu.get_pc(),
    );
    export_bus_trace_if_requested(&args.bus_trace_out, &bus);
    ExitCode::from(EXIT_PASS)
}

pub(crate) fn run_firmware(args: RunArgs) -> ExitCode {
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
        return run_firmware_arm(&args, &chip_yaml);
    }

    // RISC-V fast-boot path: load peripherals from the chip YAML and run the
    // RV32I core. This is the path used by Tier-1 fixtures for RISC-V chips
    // (e.g. ESP32-C3) which cannot go through the Xtensa boot sequence.
    if chip_yaml.contains("arch: \"riscv\"") || chip_yaml.contains("arch: riscv") {
        return run_firmware_riscv(args, chip_yaml);
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

    let mut cpu = wiring.cpu;

    // Dual-core (SMP): the APP_CPU (core 1). Created halted at the ROM reset
    // vector; released when the PRO_CPU clears CORE_1_RESETING (real hardware
    // edge, signalled via APPCPU_RESET_RELEASED). The APP_CPU then boots the
    // real ROM exactly like silicon — no firmware-symbol hooks. --rom-boot only.
    let mut cpu1: Option<labwired_core::cpu::xtensa_lx7::XtensaLx7> = None;
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
        // Bring up the APP_CPU (halted at the ROM reset vector 0x40000400).
        let mut c1 = labwired_core::cpu::xtensa_lx7::XtensaLx7::new_app_cpu();
        c1.faithful_windows = true;
        eprintln!(
            "labwired-cli run: APP_CPU created (halted at reset vector 0x{:08x})",
            c1.get_pc(),
        );
        cpu1 = Some(c1);
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

        // ESP-IDF dual-core handshake (legacy thunk-path stopgap). system_early_init
        // busy-waits until both per-core init flags are set; the single-CPU run
        // path pre-paints them. Superseded by the SMP phase of the chip model.
        let symbol_addrs = labwired_loader::extract_arduino_esp32_thunks(&elf_bytes);
        for (sym, span) in [
            ("s_cpu_inited", 2u32),
            ("s_cpu_up", 2),
            ("s_system_inited", 2),
            ("s_resume_cores", 1),
            ("s_other_cpu_startup_done", 1),
        ] {
            if let Some(&addr) = symbol_addrs.get(sym) {
                for off in 0..span {
                    let _ = bus.write_u8(addr as u64 + off as u64, 0x01);
                }
                eprintln!("labwired-cli run: handshake {sym} @0x{addr:08x} = 1");
            }
        }
    }

    // Run the step loop.
    let limit = args.max_steps.unwrap_or(u64::MAX);
    let observers: Vec<std::sync::Arc<dyn labwired_core::SimulationObserver>> = Vec::new();
    let config = labwired_core::SimulationConfig::default();
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
                        match bus.read_u32(m as u64) {
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
        let pc_before = cpu.get_pc();
        pc_ring[ring_head] = pc_before;
        ring_head = (ring_head + 1) % RING_LEN;

        // Debug breakpoint (PRO_CPU): dump on first hit.
        check_break!(cpu, pc_before, break_hit);

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
            if let Some(c1) = cpu1.as_mut() {
                c1.halted = false;
            }
            eprintln!(
                "labwired-cli run: APP_CPU released from reset → booting real ROM (step {steps})"
            );
        }

        match cpu.step(&mut bus, &observers, &config) {
            Ok(()) => {}
            Err(SimulationError::BreakpointHit(pc)) => {
                eprintln!("labwired-cli run: BREAK at 0x{pc:08x}");
                export_bus_trace_if_requested(&args.bus_trace_out, &bus);
                return ExitCode::from(EXIT_PASS);
            }
            Err(SimulationError::ExceptionRaised { cause, pc }) => {
                eprintln!("labwired-cli run: ExceptionRaised cause={cause} at 0x{pc:08x}");
                eprintln!(
                    "labwired-cli run: PS=0x{:08x} (excm={} intlevel={}) WB={} WS=0x{:04x}",
                    cpu.ps.as_raw(),
                    cpu.ps.excm(),
                    cpu.ps.intlevel(),
                    cpu.regs.windowbase(),
                    cpu.regs.windowstart(),
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
                    cpu.get_pc(),
                );
                eprintln!("labwired-cli run: a0..a15 at fault:");
                for r in 0..16u8 {
                    eprintln!("  a{:<2} = 0x{:08x}", r, cpu.regs.read_logical(r));
                }
                eprintln!(
                    "  WB=0x{:x} WS=0x{:04x}",
                    cpu.regs.windowbase(),
                    cpu.regs.windowstart(),
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
            for c in [Some(&cpu), cpu1.as_ref()].into_iter().flatten() {
                if c.get_pc() == 0x4037_e0a3 {
                    let p = c.regs.read_logical(2);
                    let mut s = String::new();
                    for i in 0..160u32 {
                        match bus.read_u8(p as u64 + i as u64) {
                            Ok(0) | Err(_) => break,
                            Ok(b) => s.push(b as char),
                        }
                    }
                    eprintln!("CCDBG: panic \"{s}\" step={steps}");
                }
            }
        }
        // Step the APP_CPU round-robin (one instruction per PRO_CPU step).
        // A halted APP_CPU returns immediately from step(). S32C1I is atomic
        // within step(), so spinlocks between the cores behave correctly.
        if let Some(c1) = cpu1.as_mut() {
            // Debug breakpoint (APP_CPU): dump on first hit.
            check_break!(c1, c1.get_pc(), break_hit1);
            match c1.step(&mut bus, &observers, &config) {
                Ok(()) | Err(SimulationError::BreakpointHit(_)) => {}
                Err(e) => {
                    eprintln!(
                        "labwired-cli run: APP_CPU error at pc=0x{:08x}: {e}",
                        c1.get_pc()
                    );
                    return ExitCode::from(EXIT_RUNTIME_ERROR);
                }
            }
        }
        bus.tick_peripherals_with_costs();
        steps += 1;

        // SMP bring-up tracer (gated). Prints both cores' PCs periodically and
        // flags the first time each core enters app XIP code (>= 0x4200_0000,
        // where setup()/loop()/Unity live) — the signal that the FreeRTOS SMP
        // scheduler finally dispatched the pinned loopTask.
        if smp_trace {
            for (core, pc) in [
                (0usize, cpu.get_pc()),
                (1usize, cpu1.as_ref().map(|c| c.get_pc()).unwrap_or(0)),
            ] {
                for w in watch.iter_mut() {
                    if w.0 == pc && !w.2[core] {
                        w.2[core] = true;
                        eprintln!("SMP: core {core} reached {} (0x{pc:08x}) step {steps}", w.1);
                    }
                }
            }
            if steps.is_multiple_of(10_000_000) {
                eprintln!(
                    "SMP: step {steps:>11}  pro=0x{:08x}  app=0x{:08x}",
                    cpu.get_pc(),
                    cpu1.as_ref().map(|c| c.get_pc()).unwrap_or(0),
                );
            }
            // Dense single-step trace window (env LABWIRED_DENSE_FROM / _LEN)
            // for following a context switch instruction-by-instruction.
            if steps >= dense_from && steps < dense_from + dense_len {
                eprintln!(
                    "D {steps} pro=0x{:08x} ps={:x} wb={} ws=0x{:04x} exc={} epc1=0x{:08x} | app=0x{:08x}",
                    cpu.get_pc(),
                    cpu.ps.as_raw(),
                    cpu.regs.windowbase(),
                    cpu.regs.windowstart(),
                    cpu.sr.read(232),
                    cpu.sr.read(177),
                    cpu1.as_ref().map(|c| c.get_pc()).unwrap_or(0),
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
                *w = bus.read_u32(base as u64 + (i * 4) as u64).unwrap_or(0);
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
    let cpu1_pc = cpu1
        .as_ref()
        .map(|c| format!(" appcpu_pc=0x{:08x}", c.get_pc()))
        .unwrap_or_default();
    eprintln!(
        "labwired-cli run: reached --max-steps {limit}; pc=0x{:08x}{cpu1_pc}",
        cpu.get_pc(),
    );
    export_bus_trace_if_requested(&args.bus_trace_out, &bus);
    ExitCode::from(EXIT_PASS)
}

pub(crate) fn run_interactive(cli: Cli) -> ExitCode {
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
    let bus = match labwired_core::system::builder::build_system_bus(system_path.as_deref()) {
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
                let chip_path = sys_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(&manifest.chip);
                match labwired_config::ChipDescriptor::from_file(&chip_path) {
                    Ok(c) => c.arch,
                    Err(e) => {
                        emit_error(
                            cli.json,
                            "ConfigError",
                            format!("Failed to parse chip descriptor: {:#}", e),
                            Some(serde_json::json!({
                                "chip_path": chip_path.display().to_string(),
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
pub(crate) fn run_firmware_arm(args: &RunArgs, chip_yaml: &str) -> ExitCode {
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
    let mut bus = match SystemBus::from_config(&chip, &manifest) {
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
    let image = match labwired_loader::load_elf(&args.firmware) {
        Ok(img) => img,
        Err(e) => {
            eprintln!("error: cannot load firmware ELF {:?}: {e}", args.firmware);
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };
    if let Err(e) = machine.load_firmware(&image) {
        eprintln!("error: cannot map firmware into bus: {e}");
        return ExitCode::from(EXIT_RUNTIME_ERROR);
    }

    // Run the step loop.
    let limit = args.max_steps.unwrap_or(u64::MAX);
    for _ in 0..limit {
        match machine.step() {
            Ok(()) => {}
            Err(e) => {
                eprintln!("labwired run (arm): simulation error: {e}");
                // Non-fatal for TIER1: the protocol may already be complete.
                break;
            }
        }
    }

    // Flush stdout.
    let _ = std::io::stdout().flush();
    export_bus_trace_if_requested(&args.bus_trace_out, &machine.bus);
    ExitCode::from(EXIT_PASS)
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

    report_metrics(&cli, &machine.cpu, &metrics);
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

    report_metrics(&cli, &machine.cpu, &metrics);
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

    report_metrics(&cli, &machine.cpu, &metrics);
    ExitCode::from(EXIT_PASS)
}
