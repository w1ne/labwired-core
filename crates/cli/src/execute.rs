// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::*;

/// Bundles the parameters `execute_test_loop` needs. Grouping them here (in
/// place of the 15+ positional arguments the loop used to take) is a pure
/// call-site/signature change: every field keeps the exact type and meaning
/// its positional counterpart had, and the loop body is otherwise untouched.
pub(crate) struct TestExecutionContext<'a, C: labwired_core::Cpu> {
    pub args: &'a TestArgs,
    pub machine: &'a mut labwired_core::Machine<C>,
    pub resolved_limits: &'a TestLimits,
    pub assertions: &'a [TestAssertion],
    pub firmware_bytes: &'a [u8],
    pub uart_tx: &'a Arc<Mutex<Vec<u8>>>,
    pub metrics: &'a Arc<labwired_core::metrics::PerformanceMetrics>,
    pub firmware_path: &'a Path,
    pub system_path: Option<&'a PathBuf>,
    pub faults: &'a [labwired_config::FaultSpec],
    pub require_fault_fired: bool,
    pub fault_evidence: Vec<labwired_cli::faults::FaultEvidence>,
    pub stimuli: &'a [labwired_config::StimulusSpec],
    pub uart_injections: &'a [labwired_config::UartInjectionSpec],
    // True when this run qualifies for the RV32IMC wasm-JIT fast path (decided
    // by `riscv_jit_test_eligible` in the caller): RiscV arch, batch mode, and
    // NONE of the per-instruction-visibility features that gate the JIT off.
    // In this mode `metrics` was NOT installed as a step observer (its presence
    // forces the JIT's correctness gate shut), so the loop mirrors the machine's
    // own counters into `metrics` before each cycle-sensitive check.
    pub jit_eligible: bool,
    // Architecture of the loaded image (paint is ARM-only in P0).
    pub arch: labwired_core::Arch,
    // Script + env kill switch for main-stack paint.
    pub stack_paint: bool,
    // Chip flash/RAM map for footprint totals and paint RAM bounds.
    pub chip_mem: Option<resource_report::ChipMemoryMap>,
    // Resolved manifest, when the run has one. Read for `cosim_models:` and the
    // directory their relative `model:` paths resolve against; `None` (a bare
    // built-in chip) declares no models, so the loop below is untouched.
    pub system: Option<&'a labwired_config::ResolvedSystem>,
}

pub(crate) fn execute_test_loop<C: labwired_core::Cpu>(
    ctx: &mut TestExecutionContext<C>,
) -> ExitCode {
    // ── Resource metrics: footprint + main-stack paint (load/reset-time) ────
    // Paint is not a SimulationObserver: fill unused stack RAM now, scan after
    // the run. Footprint is pure ELF section math and does not touch the bus.
    let footprint = resource_report::compute_footprint(ctx.firmware_bytes, ctx.chip_mem.as_ref());
    let sp_top = resource_report::arm_sp(&ctx.machine.cpu);
    let (memory_pre, paint_session) = resource_report::apply_stack_paint(
        &mut ctx.machine.bus,
        sp_top,
        ctx.arch,
        ctx.stack_paint,
        ctx.firmware_bytes,
        ctx.chip_mem.as_ref(),
    );
    // Drop load/paint bus traffic so `metrics.memory_*` reflect the run only.
    let _ = ctx.machine.bus.take_access_counts();

    // Cheap statistical PC histogram (no SimulationObserver — JIT-safe).
    let mut pc_hist: std::collections::HashMap<u32, u64> = std::collections::HashMap::new();
    let mut pc_sample_budget: u64 = 0;
    // Best-effort exception count: SimulationError::ExceptionRaised only in P1.
    let mut exception_count: u64 = 0;

    let max_steps = ctx.resolved_limits.max_steps;
    let max_cycles = ctx.resolved_limits.max_cycles;
    let max_uart_bytes = ctx.resolved_limits.max_uart_bytes;
    let detect_stuck = ctx.resolved_limits.no_progress_steps;
    let script_wall_time_ms = ctx.resolved_limits.wall_time_ms;

    let start = std::time::Instant::now();
    let mut stop_reason = StopReason::MaxSteps;
    let mut steps_executed: u64 = 0;
    // Set only when the firmware ends its own run via `simctl`; read by the
    // `firmware_exit` assertion below.
    let mut firmware_exit_code: Option<u32> = None;

    let trace_observer = if ctx.args.trace {
        let obs = Arc::new(labwired_core::trace::TraceObserver::new(
            ctx.args.trace_max.unwrap_or(100_000),
        ));
        ctx.machine.add_observer(obs.clone());
        Some(obs)
    } else {
        None
    };

    let coverage_observer = if ctx.args.coverage {
        let obs = Arc::new(labwired_core::pc_coverage::PcCoverageObserver::new());
        ctx.machine.add_observer(obs.clone());
        Some(obs)
    } else {
        None
    };

    if let Some(vcd_path) = &ctx.args.vcd {
        let file = std::fs::File::create(vcd_path).expect("Failed to create VCD file");
        let observer = std::sync::Arc::new(vcd_trace::VcdObserver::new(file));
        ctx.machine.add_observer(observer);
    }

    let mut sim_error_happened = false;
    let mut prev_pc = ctx.machine.cpu.get_pc();
    let mut stuck_counter: u64 = 0;

    // ── --watch-gpio: arm the deterministic logic-analyzer edge capture ──────
    // Resolve each `peripheral:pin` ref ONCE (to a peripheral index + pin),
    // exactly as the wasm `watch_logic_signals` accessor does, arm the in-engine
    // tap, and keep the per-channel identity so the drained edges can be shaped
    // into `result.json`'s `logic_edges` block after the run. An empty watch set
    // is a no-op (no channels installed → zero-overhead capture path).
    let logic_watch_meta: Vec<labwired_core::logic_capture::LogicChannelMeta> = {
        let refs: Vec<(String, u8)> = ctx
            .args
            .watch_gpio
            .iter()
            .filter_map(|spec| parse_watch_gpio_ref(spec))
            .collect();
        if refs.len() != ctx.args.watch_gpio.len() {
            for spec in &ctx.args.watch_gpio {
                if parse_watch_gpio_ref(spec).is_none() {
                    error!("--watch-gpio: ignoring malformed ref {spec:?} (want `peripheral:pin`)");
                }
            }
        }
        if refs.is_empty() {
            Vec::new()
        } else {
            let resolved: Vec<Option<labwired_core::logic_capture::LogicSource>> = refs
                .iter()
                .map(|(name, pin)| {
                    ctx.machine
                        .bus
                        .find_peripheral_index_by_name(name)
                        .map(|idx| labwired_core::logic_capture::LogicSource::pad(idx, *pin))
                })
                .collect();
            for ((name, _), r) in refs.iter().zip(resolved.iter()) {
                if r.is_none() {
                    error!("--watch-gpio: peripheral {name:?} not found on the bus; channel will stay flat");
                }
            }
            let initial = ctx.machine.logic_watch(&resolved);
            refs.iter()
                .zip(initial)
                .enumerate()
                .map(
                    |(ch, ((name, pin), value))| labwired_core::logic_capture::LogicChannelMeta {
                        ch: ch as u32,
                        peripheral: name.clone(),
                        pin: *pin,
                        initial: value,
                    },
                )
                .collect()
        }
    };
    let logic_capture_armed = !logic_watch_meta.is_empty();

    // ── JIT-eligible cycle/instruction sourcing (RISC-V / ESP32-C3) ──────────
    // When eligible, engage the RV32IMC wasm-JIT for this run and source the
    // metrics counters from the machine's own state (no step observer). Sourcing
    // cycles from `machine.total_cycles` (not the observer's per-step
    // `on_step_end` tap) is what makes JIT-on and JIT-off byte-identical:
    // compiled blocks retire WITHOUT firing `on_step_end`, so an observer would
    // undercount them. Both JIT arms (`LABWIRED_RISCV_JIT=1` on, default off)
    // STAY in this same machine-sourced regime, so they are byte-identical
    // (proven by tests/riscv_jit_c3_oled_test_differential); the metrics numbers
    // never depend on whether a batch was interpreted or compiled.
    if ctx.jit_eligible {
        // JIT is OPT-IN (LABWIRED_RISCV_JIT=1), NOT default-on. Measured on the
        // esp32c3-oled-demo oracle lab, the wasmtime RV32IMC JIT is ~18× SLOWER
        // than the interpreter here: the hot path is tight FreeRTOS/idle loops
        // (~1.9 guest instr per compiled-block run), so the per-block-dispatch
        // FFI overhead dwarfs the interpreted cost and ~⅔ of instructions still
        // fall back to the interpreter. The genuine speedup on this path is the
        // tick-interval widening below (`Machine::advance` at the bus max-safe
        // interval: ~2.6× faster than the pre-change single-step tick-1 oracle),
        // which is applied UNCONDITIONALLY when eligible. The JIT stays wired,
        // proven byte-identical, and one env var away for compute-heavy firmware
        // where straight-line blocks amortize the dispatch cost. See the report.
        let jit_on = std::env::var("LABWIRED_RISCV_JIT").as_deref() == Ok("1");
        ctx.machine.config.riscv_jit_enabled = jit_on;
        ctx.machine.bus.config.riscv_jit_enabled = jit_on;
        // Widen the peripheral-tick interval to RECOMMENDED_TICK_INTERVAL so
        // `Machine::advance`'s per-tick batch is wide enough
        // for compiled blocks to retire, and the peripheral tick count drops
        // ~64×. The C3 rom-boot peripherals are walk-deletable, so this is
        // observably identical to interval-1 (esp32c3_walk_differential); the
        // eligibility gate already excludes any `requires_cycle_accurate` bus.
        // `max_safe_tick_interval` is NOT used here because it only returns the
        // wide interval under the `event-scheduler` feature, which the CLI does
        // not enable (see crates/cli/Cargo.toml). Crucially this is applied to
        // BOTH JIT arms, so it never perturbs the JIT-on vs JIT-off differential.
        // TEST-ONLY escape hatch (regression gate riscv_jit_c3_oled_test_differential):
        // override the widened interval with LABWIRED_TICK_INTERVAL so the
        // interval-64 (widened) vs interval-1 (baseline) fidelity gate can be
        // proven empirically with EVERYTHING else identical (same machine-sourced
        // cycle counting, same eligible code path) — the tick interval is the ONLY
        // variable. Unset = default (RECOMMENDED_TICK_INTERVAL).
        let interval = std::env::var("LABWIRED_TICK_INTERVAL")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(labwired_core::bus::RECOMMENDED_TICK_INTERVAL);
        ctx.machine.config.peripheral_tick_interval = interval;
        ctx.machine.bus.config.peripheral_tick_interval = interval;
    }

    let otherwise_batch_eligible = ctx.machine.config.batch_mode_enabled
        && ctx.args.breakpoint.is_empty()
        && detect_stuck.is_none()
        // Cycle-tight GPIO-timing devices (e.g. HC-SR04 ECHO pulse) only behave
        // correctly when peripherals tick between every instruction; instruction
        // batching freezes them across the batch and the firmware measures 0.
        // Push-mode channels report their own edges from the write sites and keep
        // the full batch width.
        && !ctx.machine.logic_poll_active();
    let batch_size = assertion_observation_batch_size(
        otherwise_batch_eligible,
        ctx.resolved_limits.stop_when_assertions_pass,
        ctx.assertions,
        max_steps,
    );

    // ── Fuel budget vs CPU batch width ──────────────────────────────────────
    // These are two different knobs that this loop used to collapse into one
    // number, and the collapse silently disarmed idle fast-forward.
    //
    // `batch_cap` is how many instructions the CPU may retire per window. It
    // stays `batch_size`, which is 1 whenever batch mode is off (the ESP32-C3
    // rom-boot path turns batching off because a fixed-width batch freezes
    // interrupt delivery and FreeRTOS never runs). That is a FIDELITY setting
    // and is not touched here or below.
    //
    // The advance call's `fuel` is a different thing: how much this call may
    // consume before returning to the checks at the top of this loop. And
    // `Machine::try_idle_fast_forward` clamps its skip to the fuel remaining —
    // so with fuel pinned to 1, an idle FreeRTOS window was fast-forwarded ONE
    // cycle at a time. Measured on a hosted-shaped ESP32-C3 BLE rom-boot run,
    // turning the flag on that way bought 3% of the steps and ran ~2x SLOWER in
    // wall clock than not skipping at all, because every skipped cycle paid a
    // full plan/commit round trip. A `vTaskDelay(200)` window is ~32M cycles;
    // it wants to go in a few thousand skips, not 32M of them.
    //
    // The widened fuel below is applied ONLY on an iteration where the CPU is
    // already parked waiting for an interrupt (`idle_fast_forward_budget` is
    // `Some`). That is the tight guard: while the CPU is parked the advance
    // call retires no instructions at all, so nothing this loop observes
    // between calls — PC, retired-step counts, assertion settling — can move
    // inside the widened window. On every instruction-retiring iteration the
    // fuel is exactly what it was before this change, so a busy run is
    // unchanged instruction for instruction.
    //
    // `idle_ff_wide_observation` is the standing half of the condition. It
    // excludes the features this loop — not `advance` — implements per
    // iteration and which a parked CPU does not exempt:
    //   * `--breakpoint` / `--detect-stuck` re-read the PC between calls, and
    //     a WFI spin is exactly a stuck PC,
    //   * `--capture-app-entry` watches for the app-entry PC between calls,
    //   * poll-mode logic capture and ShutdownLatency assertions need
    //     cycle-accurate attribution of events inside the window.
    // Time-triggered stimuli, UART injections and `max_cycles` are NOT in that
    // list, and they do not turn fast-forward off either. Every one of them is
    // compared against `machine.total_cycles`, the machine clock an idle skip
    // advances too, and the per-iteration cap below hands `advance` a
    // simulated-cycle limit that ends exactly on the next threshold. An idle
    // skip is clamped to that limit like any other work, so a threshold lands
    // on its cycle whether the CPU was busy or parked.
    //
    // (They used to disable fast-forward outright: the thresholds were then
    // compared against the `PerformanceMetrics` counter, which an idle skip
    // does not advance, so a skip moved every stimulus late. That counter is
    // now a performance figure only.)
    //
    // With idle fast-forward off — including via `LABWIRED_IDLE_FAST_FORWARD=0`
    // — this is `false` and the loop is byte-identical to before.

    // The `event-scheduler` clause is load-bearing, not belt-and-braces. Without
    // that feature `Machine::try_idle_fast_forward` is compiled to `0`, so there
    // is no skip for the wider fuel to fund — but `idle_fast_forward_budget`
    // still reports the parked CPU, and widening on that would hand `advance` a
    // million instructions of WFI spin to retire in one call. The outer loop's
    // per-iteration checks (`stop_when_assertions_pass` settling,
    // `max_uart_bytes`, `wall_time_ms`) would then run a million steps apart in
    // a DEFAULT build. Gated here, a build without the feature takes the
    // `else` arm exactly as it does today.
    let idle_ff_wide_observation = cfg!(feature = "event-scheduler")
        && ctx.machine.config.idle_fast_forward_enabled
        && ctx.args.breakpoint.is_empty()
        && detect_stuck.is_none()
        && ctx.args.capture_app_entry.is_none()
        && !ctx.machine.logic_poll_active()
        && !requires_fine_grained_observation(ctx.assertions);

    // ── Co-simulation: step manifest `cosim_models` in lockstep ─────────────
    //
    // Built ONLY when the manifest declares models. Without them `cosim` is
    // `None`, nothing below it runs, and the advance request is byte-identical
    // to what it was before co-simulation existed — a manifest with no
    // `cosim_models:` cannot pay for this.
    //
    // Lockstep, not "eventually": the request's simulated-cycle budget is
    // clamped to the cycles left before the next model boundary, so the machine
    // can never run PAST a boundary and then hand a model pin levels from its
    // future. Everything is synchronous — no threads, no wall clock — so the
    // same firmware produces the same model inputs on every run.
    let mut cosim = match ctx.system {
        Some(system) => {
            match labwired_core::cosim::CosimSession::new(
                &system.manifest.cosim_models,
                system.base_dir(),
                &ctx.machine.bus,
            ) {
                Ok(session) => session,
                Err(e) => {
                    error!("co-sim: failed to start the declared models: {e}");
                    return ExitCode::from(EXIT_RUNTIME_ERROR);
                }
            }
        }
        None => None,
    };
    if let Some(session) = &cosim {
        // An unresolvable path fails the run rather than degrading it. The
        // whole point of routing a pin into a model is that the model sees the
        // pin; a run that silently read nothing would still print a verdict,
        // and that verdict would be evidence of nothing. Same rule the
        // declarative stimuli follow when a channel does not resolve.
        if !session.binding_errors().is_empty() {
            for err in session.binding_errors() {
                error!("{err}");
            }
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
        if session.uses_fallback_clock() {
            warn!(
                "co-sim: this bus reports no core clock; assuming {} Hz for the model time base",
                session.cpu_hz()
            );
        }
        info!(
            "co-sim: {} model(s), stepping every {} ns at {} Hz",
            session.model_count(),
            session.step_ns(),
            session.cpu_hz()
        );
        // Publish the waveform ring the session's analog models fill, so
        // `--analog-trace` and `Machine::analog_trace_snapshot` read the samples
        // this run actually produces instead of an unattached, header-only trace.
        ctx.machine
            .attach_analog_trace(session.analog_trace_registry());
    }

    // Declarative stimuli (schema_version 1.2). A device input is applied via
    // the generic `Machine::set_input` path (see `labwired_core::sim_input`), so
    // no per-type wiring; a `cosim_signal` goes through the co-simulation
    // session built above, the same `set_signal_number` the browser bridge
    // calls. `at_start` fires now; `after_cycles` fires the first loop
    // iteration at or past its cycle threshold. The closure takes `machine` and
    // the session as arguments (captures nothing) so it can be called both here
    // and mid-loop.
    //
    // The closure RETURNS the outcome rather than swallowing it. This used to
    // only `error!` a rejection into the log and carry on, so a run whose input
    // never reached the device still reported `status: "pass"` — a surface that
    // claims success having proved nothing. Every outcome is now recorded in
    // `stimulus_outcomes`, surfaced in `result.json`'s `stimuli` block, and a
    // rejection fails the run (see the verdict below).
    let mut stimulus_outcomes: Vec<StimulusOutcome> = Vec::new();
    let apply_stimulus = |machine: &mut labwired_core::Machine<C>,
                          cosim: &mut Option<labwired_core::cosim::CosimSession>,
                          s: &labwired_config::StimulusSpec| {
        let (channel, result) = match &s.action {
            labwired_config::StimulusAction::Input { target, value } => {
                let result = match target.component.as_deref() {
                    Some(component) => machine.set_input_on(component, &target.channel, *value),
                    None => machine.set_input(&target.channel, *value),
                };
                // `SimInputError`'s Display is the author-facing sentence ("no
                // attached input device exposes channel 'pressed'"); the old
                // `{:?}` Debug form leaked Rust variant names into the log.
                (&target.channel, result.map_err(|e| e.to_string()))
            }
            labwired_config::StimulusAction::CosimSignal(signal) => {
                let result = match cosim.as_mut() {
                    Some(session) => session
                        .set_signal_number(&signal.path, signal.value)
                        .map_err(|e| e.to_string()),
                    None => Err(format!(
                        "co-sim signal '{}': this run declares no cosim_models, so nothing reads it",
                        signal.path
                    )),
                };
                (&signal.path, result)
            }
        };
        let (outcome, error) = match result {
            Ok(()) => {
                info!("stimulus: {} = {} applied", channel, s.value());
                (artifacts::STIMULUS_APPLIED, None)
            }
            Err(e) => {
                error!(
                    "stimulus '{}' = {} could not be applied: {e}",
                    channel,
                    s.value()
                );
                (artifacts::STIMULUS_REJECTED, Some(e))
            }
        };
        StimulusOutcome::new(s, outcome, machine.total_cycles, error)
    };
    let mut stimulus_cycles: StimulusCycles = std::collections::HashMap::new();
    let mut stimulus_sequence = 0u64;
    let mut uart_milestone_cycles =
        UartMilestoneCycles::new(ctx.assertions.iter().filter_map(|a| {
            if let TestAssertion::ShutdownLatency(a) = a {
                Some(a.shutdown_latency.to_uart.clone())
            } else {
                None
            }
        }));
    for s in ctx.stimuli {
        if matches!(s.trigger, labwired_config::FaultTrigger::AtStart) {
            let outcome = apply_stimulus(ctx.machine, &mut cosim, s);
            if let (None, Some(target)) = (&outcome.error, s.input_target()) {
                stimulus_sequence += 1;
                stimulus_cycles
                    .entry(stimulus_key(target))
                    .or_default()
                    .push(StimulusApplication {
                        cycle: ctx.machine.total_cycles,
                        value: s.value(),
                        sequence: stimulus_sequence,
                    });
            }
            stimulus_outcomes.push(outcome);
        }
    }
    // Time-triggered stimuli, each tagged with whether it has fired yet.
    let mut pending_stimuli: Vec<(&labwired_config::StimulusSpec, bool)> = ctx
        .stimuli
        .iter()
        .filter(|s| matches!(s.trigger, labwired_config::FaultTrigger::AfterCycles { .. }))
        .map(|s| (s, false))
        .collect();

    // Declarative UART RX injections (schema_version 1.2). Resolved against
    // the built bus by peripheral name via the same `attach_uart_rx_source`
    // family used by the wasm bridge (`attach_uart_rx_source_named`), then
    // pushed straight into the UART's RX `VecDeque` — the shared mechanism
    // that already backs interactive serial input. A byte pushed before the
    // firmware configures/reads the UART is buffered, not dropped: RX
    // presence is derived from the queue being non-empty (see
    // `Uart::read`), with no enable-bit gating. `at_start` delivers
    // immediately (before the firmware executes its first instruction);
    // `after_cycles` delivers the first loop iteration at or past its cycle
    // threshold, mirroring `apply_stimulus` above. A named UART that isn't
    // found on the bus is a hard config error — silently dropping serial
    // input a script depends on would be a false pass.
    let mut uart_injection_error = false;
    let apply_uart_injection =
        |machine: &mut labwired_core::Machine<C>, u: &labwired_config::UartInjectionSpec| {
            match machine.bus.attach_uart_rx_source_named(&u.uart) {
                Some(rx) => {
                    let bytes = u.bytes.as_bytes();
                    match rx.lock() {
                        Ok(mut guard) => {
                            guard.extend(bytes.iter().copied());
                            info!(
                                "uart_injection: {} byte(s) delivered to '{}'",
                                bytes.len(),
                                u.uart
                            );
                        }
                        Err(e) => error!("uart_injection '{}': RX buffer poisoned: {e}", u.uart),
                    }
                    None
                }
                None => Some(format!(
                    "uart_injection: UART peripheral '{}' not found on the bus",
                    u.uart
                )),
            }
        };
    for u in ctx.uart_injections {
        if matches!(u.trigger, labwired_config::FaultTrigger::AtStart) {
            if let Some(err) = apply_uart_injection(ctx.machine, u) {
                error!("{err}");
                uart_injection_error = true;
            }
        }
    }
    if uart_injection_error {
        return ExitCode::from(EXIT_CONFIG_ERROR);
    }
    let mut pending_uart_injections: Vec<(&labwired_config::UartInjectionSpec, bool)> = ctx
        .uart_injections
        .iter()
        .filter(|u| matches!(u.trigger, labwired_config::FaultTrigger::AfterCycles { .. }))
        .map(|u| (u, false))
        .collect();

    // Tracks the step at which all runtime assertions first passed. The
    // `stop_when_assertions_pass` early-stop is only accepted after the machine
    // keeps executing for a settling window past this point WITHOUT faulting —
    // print-then-crash firmware breaks with its fault reason during the window
    // instead of certifying as passed. A regression (assertions stop passing)
    // resets it, so the pass must be durable.
    let mut assertions_first_passed_at: Option<u64> = None;
    // Milestone assertions describe state observed before a later injected
    // fault. Once the requested speed band has genuinely been observed in the
    // plant, retain that evidence through the shutdown phase.
    let mut assertion_latched = vec![false; ctx.assertions.len()];

    // ── --capture-app-entry: cache a genuine faithful-boot state ─────────
    // While the REAL rom-boot runs, snapshot the machine the instant control
    // first reaches the application, write the `.lwrs`, then keep running so
    // this same cold invocation still emits the normal evidence. The capture
    // point is a real mid-flight boot state — NOT a hand-modeled handoff.
    struct AppEntryCapture {
        path: PathBuf,
        chip: &'static str,
        fw_sha: [u8; 32],
        // App-entry PC resolved from the ELF (`call_start_cpu0`, else
        // `app_main`); `None` falls back to the XIP app-window detector.
        target_pc: Option<u32>,
    }
    let mut app_entry_capture: Option<AppEntryCapture> =
        ctx.args.capture_app_entry.as_ref().and_then(|path| {
            let Some((chip, fw_sha)) = rom_boot_flash_self_key() else {
                error!(
                    "--capture-app-entry needs a faithful rom-boot (set LABWIRED_ESP32C3_FLASH \
                     or LABWIRED_ESP32S3_FLASH); skipping capture"
                );
                return None;
            };
            let target_pc =
                labwired_loader::resolve_symbol_in_elf(ctx.firmware_bytes, "call_start_cpu0")
                    .or_else(|| {
                        labwired_loader::resolve_symbol_in_elf(ctx.firmware_bytes, "app_main")
                    });
            match target_pc {
                Some(pc) => {
                    info!("capture-app-entry: chip={chip} app-entry PC 0x{pc:08x} (ELF symbol)")
                }
                None => info!(
                    "capture-app-entry: chip={chip} no call_start_cpu0/app_main symbol; \
                     using first PC in XIP app window [0x42000000,0x44000000)"
                ),
            }
            Some(AppEntryCapture {
                path: path.clone(),
                chip,
                fw_sha,
                target_pc,
            })
        });

    // ── stop_when_assertions_pass: per-step evaluation, made cheap ──────────
    //
    // The early-stop pins the CLI batch to one instruction (`batch_size`
    // above), so the block at the bottom of this loop runs once per RETIRED
    // GUEST INSTRUCTION. It used to copy the entire UART capture TWICE every
    // time — `Vec::clone`, then `String::from_utf8_lossy(..).to_string()` —
    // and re-scan the result for every assertion. MEASURED on the pinned C3
    // Arduino-BLE image (`e2e_esp32c3_ble_arduino`), 20 M steps: 5.46 s user
    // CPU with the scan, 2.16 s with the identical run and nothing to scan.
    // 60 % of the run was re-reading a 235-byte buffer that only changes a few
    // hundred times in 362 M steps, and `from_utf8_lossy(..).to_string()`
    // alone was 37 % of process samples.
    //
    // The verdict is a pure function of (uart_text, machine state), so it is
    // recomputed EXACTLY when one of those can have changed:
    //
    //   * `uart_text` changes only when the sink grows — a UART TX sink is
    //     append-only (nothing in this file or the core UART models truncates
    //     it; it is drained once, after the loop, to write `uart.log`), so its
    //     length is an exact change detector;
    //   * machine state changes every step, so any assertion that READS the
    //     machine (`memory_value`, `uds_tester`) still forces a recompute
    //     every step — those keep their old cost, which is the honest price of
    //     asking a question about live machine state.
    //
    // When every runtime assertion is UART-only (what every `uart_contains` /
    // `uart_regex` script is, including both BLE gates), an unchanged length
    // means an unchanged verdict and the cached one is reused. The verdict is
    // therefore identical on every step, so `assertions_first_passed_at`
    // latches on the same step and the run stops at the same step count.
    //
    // The cache is DELIBERATELY conservative about which assertion kinds count
    // as UART-only: `MotorSpeedReached` latches milestones and
    // `ShutdownLatency` reads `stimulus_cycles`/`uart_milestone_cycles`, both
    // of which move without the capture growing, so their presence disables
    // the cache and restores the original every-step evaluation.
    let has_runtime_assertions = ctx.assertions.iter().any(|a| {
        !matches!(
            a,
            TestAssertion::ExpectedStopReason(_)
                | TestAssertion::FirmwareExit(_)
                | TestAssertion::ResourceBudget(_)
        )
    });
    let assertions_are_uart_only = ctx
        .assertions
        .iter()
        .filter(|a| {
            !matches!(
                a,
                TestAssertion::ExpectedStopReason(_)
                    | TestAssertion::FirmwareExit(_)
                    | TestAssertion::ResourceBudget(_)
            )
        })
        .all(|a| {
            matches!(
                a,
                TestAssertion::UartContains(_) | TestAssertion::UartRegex(_)
            )
        });
    let mut cached_uart_text = String::new();
    // `usize::MAX` (not 0) so the first iteration always counts as a change and
    // evaluates, even when the capture is still empty.
    let mut cached_uart_len = usize::MAX;
    let mut cached_all_pass = false;

    let mut step = 0;
    while step < max_steps {
        // JIT-eligible path: mirror the machine's authoritative counters into
        // `metrics` BEFORE the cycle-sensitive checks below (stimulus
        // `after_cycles`, `max_cycles`), so they fire at exactly the same batch
        // boundary the observer path would. `step` is the retired-instruction
        // count (accumulated from `step_batch` return values); `total_cycles`
        // is the machine's canonical cycle counter. No-op for the non-eligible
        // path, where `metrics` IS the live step observer.
        if ctx.jit_eligible {
            ctx.metrics.set_cycles(ctx.machine.total_cycles);
            ctx.metrics.set_instructions(step);
        }
        // --capture-app-entry: detect the first instant execution reaches the
        // application, snapshot the live machine, and write the resume blob.
        if let Some(cap) = &app_entry_capture {
            let pc = ctx.machine.cpu.get_pc();
            let reached = cap.target_pc == Some(pc) || (0x4200_0000..0x4400_0000).contains(&pc);
            if reached {
                // Same reason as `snapshot capture`: a CPU that models no
                // runtime snapshot answers `None`, and writing a resume file
                // without a CPU half would produce something that still looks
                // valid. Say so and write nothing. NOT a `continue` — the rest
                // of this loop body is what actually advances the machine, so
                // skipping it would hang the run instead of just declining the
                // capture.
                if let Some(mut snap) = ctx.machine.take_runtime_snapshot() {
                    snap.set_self_key(cap.chip, cap.fw_sha);
                    if let Some(parent) = cap.path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    match std::fs::write(&cap.path, snap.to_bytes()) {
                        Ok(()) => info!(
                            "capture-app-entry: snapshot written to {:?} at app-entry pc=0x{pc:08x} \
                             (cold-boot step {step})",
                            cap.path
                        ),
                        Err(e) => error!("capture-app-entry: failed to write {:?}: {e}", cap.path),
                    }
                } else {
                    error!(
                        "capture-app-entry: this CPU has no runtime-snapshot implementation \
                         (supported: RISC-V, Xtensa LX7) — no snapshot written to {:?}",
                        cap.path
                    );
                }
                // Capture once; keep running so the cold invocation still
                // produces the normal serial/cycle evidence.
                app_entry_capture = None;
            }
        }
        // Fire any `after_cycles` stimulus whose threshold the run has reached.
        if !pending_stimuli.is_empty() {
            let cycles = ctx.machine.total_cycles;
            for (s, fired) in pending_stimuli.iter_mut() {
                if *fired {
                    continue;
                }
                if let labwired_config::FaultTrigger::AfterCycles { cycles: threshold } = s.trigger
                {
                    if cycles >= threshold {
                        let outcome = apply_stimulus(ctx.machine, &mut cosim, s);
                        if let (None, Some(target)) = (&outcome.error, s.input_target()) {
                            stimulus_sequence += 1;
                            stimulus_cycles
                                .entry(stimulus_key(target))
                                .or_default()
                                .push(StimulusApplication {
                                    cycle: ctx.machine.total_cycles,
                                    value: s.value(),
                                    sequence: stimulus_sequence,
                                });
                        }
                        stimulus_outcomes.push(outcome);
                        *fired = true;
                    }
                }
            }
        }
        // Fire any `after_cycles` UART injection whose threshold has been reached.
        if !pending_uart_injections.is_empty() {
            let cycles = ctx.machine.total_cycles;
            for (u, fired) in pending_uart_injections.iter_mut() {
                if *fired {
                    continue;
                }
                if let labwired_config::FaultTrigger::AfterCycles { cycles: threshold } = u.trigger
                {
                    if cycles >= threshold {
                        if let Some(err) = apply_uart_injection(ctx.machine, u) {
                            error!("{err}");
                        }
                        *fired = true;
                    }
                }
            }
        }
        if !ctx.args.breakpoint.is_empty()
            && ctx.args.breakpoint.contains(&ctx.machine.cpu.get_pc())
        {
            stop_reason = StopReason::Halt;
            steps_executed = step;
            break;
        }
        if let Some(wall_time_ms) = script_wall_time_ms {
            if start.elapsed().as_millis() >= wall_time_ms as u128 {
                stop_reason = StopReason::WallTime;
                break;
            }
        }

        // Check max_cycles
        if let Some(limit) = max_cycles {
            if ctx.machine.total_cycles >= limit {
                stop_reason = StopReason::MaxCycles;
                break;
            }
        }

        // Check max_uart_bytes
        if let Some(limit) = max_uart_bytes {
            let current_len = ctx.uart_tx.lock().map(|g| g.len() as u64).unwrap_or(0);
            if current_len >= limit {
                stop_reason = StopReason::MaxUartBytes;
                break;
            }
        }

        let remaining = (max_steps - step) as u32;
        let current_batch = batch_size as u32;
        let to_execute = current_batch.min(remaining);

        let (mut limit, batch_cap) = if ctx.jit_eligible {
            let chunk = remaining.min(JIT_RUN_CHUNK);
            (u64::from(chunk), chunk)
        } else if idle_ff_wide_observation
            && assertions_first_passed_at.is_none()
            && ctx
                .machine
                .cpu
                .idle_fast_forward_budget(&ctx.machine.bus as &dyn labwired_core::Bus)
                .is_some()
        {
            // CPU is parked on WFI right now: give the skip real fuel so the
            // idle window goes in a few thousand skips instead of one cycle at
            // a time. The CPU batch width is still `current_batch` — see the
            // note beside `idle_ff_wide_observation`.
            //
            // Once `stop_when_assertions_pass` has latched, this arm stays
            // OFF. Settle is "N more instructions" (print-then-bkpt). Wide
            // idle-ff would skip to the next RTC overflow (~512 s on
            // nRF52840) because WFI does not retire those N steps.
            (u64::from(remaining.min(IDLE_FF_RUN_CHUNK)), current_batch)
        } else {
            (u64::from(to_execute), current_batch)
        };
        let current_cycle = ctx.machine.total_cycles;
        let mut cycle_cap: Option<u64> = None;
        let mut cap_at = |threshold: u64| {
            if threshold > current_cycle {
                let distance = threshold - current_cycle;
                cycle_cap = Some(cycle_cap.map_or(distance, |cap| cap.min(distance)));
            }
        };
        for (stimulus, fired) in &pending_stimuli {
            if !*fired {
                if let labwired_config::FaultTrigger::AfterCycles { cycles } = stimulus.trigger {
                    cap_at(cycles);
                }
            }
        }
        for (injection, fired) in &pending_uart_injections {
            if !*fired {
                if let labwired_config::FaultTrigger::AfterCycles { cycles } = injection.trigger {
                    cap_at(cycles);
                }
            }
        }
        if let Some(cycle_limit) = max_cycles {
            cap_at(cycle_limit);
        }
        if let Some(cap) = cycle_cap {
            limit = limit.min(cap);
        }
        let mut request = labwired_core::AdvanceRequest::run(Some(limit.max(1)))
            .with_batch_cap(
                std::num::NonZeroU32::new(batch_cap.max(1)).expect("advance batch cap is non-zero"),
            )
            .with_breakpoints(labwired_core::BreakpointPolicy::Ignore);
        if let Some(cycles_to_trigger) = cycle_cap {
            request = request.with_cycle_limit(cycles_to_trigger);
        }
        // With co-simulation models the session advances the machine: it stops
        // the machine ON the next model boundary, never past it, then samples
        // the routed pins, steps every model due and writes the routed outputs
        // back, so the firmware's next instruction sees the model's answer to
        // the levels it had just driven. Without models this is the plain
        // `Machine::advance` the loop always issued.
        let advanced = match cosim.as_mut() {
            Some(session) => session.advance(ctx.machine, request),
            None => ctx
                .machine
                .advance(request)
                .map(labwired_core::cosim::CosimAdvance::from)
                .map_err(labwired_core::cosim::CosimAdvanceError::Machine),
        };
        let (report, boundary) = match advanced {
            Ok(advance) => (
                advance.report,
                Ok((advance.routed, advance.new_routing_errors)),
            ),
            Err(labwired_core::cosim::CosimAdvanceError::Model { report, error }) => {
                (report, Err(error))
            }
            Err(labwired_core::cosim::CosimAdvanceError::Machine(error)) => {
                sim_error_happened = true;
                if matches!(
                    error,
                    labwired_core::SimulationError::ExceptionRaised { .. }
                ) {
                    exception_count = exception_count.saturating_add(1);
                }
                stop_reason = map_sim_error_to_stop_reason(&error);
                if stop_reason != StopReason::Halt {
                    error!("Simulation error at step {}: {}", step, error);
                }
                break;
            }
        };

        step += report.primary_steps;
        steps_executed = step;
        // Statistical PC sampling: one histogram hit every
        // PC_SAMPLE_EVERY retired primary steps, using the post-batch
        // PC (no per-instruction observer — keeps JIT eligible).
        if report.primary_steps > 0 {
            pc_sample_budget = pc_sample_budget.saturating_add(report.primary_steps);
            while pc_sample_budget >= resource_report::PC_SAMPLE_EVERY {
                pc_sample_budget -= resource_report::PC_SAMPLE_EVERY;
                resource_report::note_pc_sample(&mut pc_hist, ctx.machine.cpu.get_pc());
            }
        }
        // A firmware-authored verdict ends the run immediately: the
        // firmware has stated the result, so continuing would only let
        // a later timeout overwrite it.
        if let labwired_core::AdvanceStop::FirmwareExit { code } = report.stop {
            info!("{} (step={})", firmware_exit_message(code), step);
            stop_reason = StopReason::FirmwareExit;
            firmware_exit_code = Some(code);
            break;
        }
        if report.primary_steps == 0 && report.idle_cycles == 0 {
            stop_reason = StopReason::Halt;
            break;
        }

        // ── Co-simulation boundary ──────────────────────────────────────────
        // What the session did at the boundary it stopped the machine on.
        // Empty when no model is declared or none was due.
        match boundary {
            Ok((routed, errors)) => {
                if let Some(session) = &cosim {
                    if !routed.is_empty() {
                        for (path, value) in session.sampled_inputs() {
                            debug!(
                                target: "cosim",
                                "{} = {} (cycle={}) -> models",
                                path,
                                value,
                                ctx.machine.total_cycles,
                            );
                        }
                    }
                }
                for model_step in &routed {
                    for (path, value) in &model_step.outputs {
                        debug!(
                            target: "cosim",
                            "{} -> {} = {} (cycle={})",
                            model_step.model_id,
                            path,
                            value,
                            ctx.machine.total_cycles,
                        );
                    }
                }
                // A routing failure that only shows up mid-run (an ADC that
                // refuses a channel) comes back once per distinct failure, not
                // once per co-simulation step.
                for err in errors {
                    error!("{err}");
                }
            }
            Err(e) => {
                // The external model is half of this simulation. Carrying
                // on without it would run the firmware against a plant that
                // stopped answering and still call the result a verdict.
                error!(
                    "co-sim step failed at cycle {}: {e}",
                    ctx.machine.total_cycles
                );
                sim_error_happened = true;
                stop_reason = StopReason::Exception;
                break;
            }
        }

        // Check no_progress (PC stuck) - only if batching disabled or not possible
        if let Some(limit) = detect_stuck {
            let current_pc = ctx.machine.cpu.get_pc();
            if current_pc == prev_pc {
                stuck_counter += 1;
                if stuck_counter >= limit {
                    stop_reason = StopReason::NoProgress;
                    error!(
                        "No progress (PC stuck at {:#x}) for {} steps",
                        prev_pc, limit
                    );
                    break;
                }
            } else {
                stuck_counter = 0;
                prev_pc = current_pc;
            }
        }

        if !uart_milestone_cycles.occurrences.is_empty() {
            let uart_bytes = ctx.uart_tx.lock().map(|g| g.clone()).unwrap_or_default();
            uart_milestone_cycles.observe(&uart_bytes, ctx.machine.total_cycles);
        }

        if ctx.resolved_limits.stop_when_assertions_pass && has_runtime_assertions {
            // Refresh the cached capture only when the sink actually grew.
            // One uncontended lock + a length compare on the common step.
            let uart_changed = match ctx.uart_tx.lock() {
                Ok(g) => {
                    if g.len() != cached_uart_len {
                        cached_uart_len = g.len();
                        cached_uart_text.clear();
                        cached_uart_text.push_str(&String::from_utf8_lossy(&g[..]));
                        true
                    } else {
                        false
                    }
                }
                // Poisoned mutex: the old code read this as an empty
                // capture (`unwrap_or_default`). Reproduce that, once.
                Err(_) => {
                    if cached_uart_len != 0 {
                        cached_uart_len = 0;
                        cached_uart_text.clear();
                        true
                    } else {
                        false
                    }
                }
            };
            if uart_changed || !assertions_are_uart_only {
                let uart_text = &cached_uart_text;
                for (index, assertion) in ctx.assertions.iter().enumerate() {
                    let milestone_observed = match assertion {
                        TestAssertion::MotorSpeedReached(_) => {
                            assertion_currently_passes(assertion, uart_text, ctx.machine)
                        }
                        _ => false,
                    };
                    if milestone_observed {
                        assertion_latched[index] = true;
                    }
                }
                cached_all_pass = ctx.assertions.iter().enumerate().all(|(index, assertion)| {
                    matches!(
                        assertion,
                        TestAssertion::ExpectedStopReason(_)
                            | TestAssertion::FirmwareExit(_)
                            | TestAssertion::ResourceBudget(_)
                    ) || (matches!(assertion, TestAssertion::MotorSpeedReached(_))
                        && assertion_latched[index])
                        || matches!(assertion, TestAssertion::ShutdownLatency(a)
                        if shutdown_latency_passes(
                            &a.shutdown_latency,
                            &stimulus_cycles,
                            &uart_milestone_cycles,
                        ))
                        || assertion_currently_passes(assertion, uart_text, ctx.machine)
                });
            }
            let all_pass = cached_all_pass;
            if all_pass {
                // Latch the first all-pass step, but not before the absolute
                // minimum-steps floor: assertions that satisfy trivially early
                // (e.g. a token already present at reset) don't short-circuit
                // the run before real execution has happened.
                if assertions_first_passed_at.is_none()
                    && step >= ctx.resolved_limits.stop_when_assertions_pass_min_steps
                {
                    assertions_first_passed_at = Some(step);
                    // Settle is "N more instructions" (print-then-bkpt). Leave
                    // idle-ff on and a parked WFI never retires those N steps
                    // — it skips to the next RTC overflow instead (~512 s on
                    // nRF52840). Interpreting WFI for the window is cheap and
                    // keeps the crash-during-settle contract.
                    ctx.machine.config.idle_fast_forward_enabled = false;
                }
            } else {
                // A regression means the pass was not durable — restart the
                // settling window from scratch.
                assertions_first_passed_at = None;
            }
            if let Some(first) = assertions_first_passed_at {
                if step.saturating_sub(first)
                    >= ctx.resolved_limits.stop_when_assertions_pass_settle_steps
                {
                    stop_reason = StopReason::AssertionsPassed;
                    break;
                }
            }
        }
    }

    // Final counter mirror for the JIT-eligible path: the loop-top sync runs
    // before the LAST batch, so capture that batch's retired cycles/instructions
    // here — `result.json` (`cycles`/`instructions`) and `stop_reason_details`
    // read `metrics` below and must report the true totals.
    if ctx.jit_eligible {
        ctx.metrics.set_cycles(ctx.machine.total_cycles);
        ctx.metrics.set_instructions(steps_executed);
    }

    // Opt-in JIT non-vacuity / diagnostic: prove hot blocks actually compiled
    // and ran on this oracle run (LABWIRED_JIT_STATS=1). `jit_engine_stats` is a
    // feature-agnostic Cpu-trait accessor: `Some(..)` only in a `jit-core` build
    // whose JIT engine was created, `None` otherwise (interpreter-only).
    if ctx.jit_eligible && std::env::var("LABWIRED_JIT_STATS").is_ok() {
        match ctx.machine.cpu.jit_engine_stats() {
            Some(s) => eprintln!(
                "[jit-stats] compiled={} block_runs={} block_instrs={} interpreted={}",
                s.compiled, s.block_runs, s.block_instrs, s.interpreted
            ),
            None => eprintln!("[jit-stats] JIT engine never created (interpreter-only run)"),
        }
    }

    // How much of this run's device time the CPU spent parked and skipped
    // rather than interpreted. Printed whenever it is non-zero so a hosted run
    // can be shown to have actually fast-forwarded — the failure mode this
    // guards is a build or a run path where the flag is on and the skip is
    // clamped to nothing, which is indistinguishable from working unless the
    // number is visible. `steps_executed + skipped == machine.total_cycles`.
    if ctx.machine.idle_fast_forward_cycles_skipped > 0 {
        eprintln!(
            "labwired-cli test: idle_ff skipped {} of {} device cycles ({} interpreted)",
            ctx.machine.idle_fast_forward_cycles_skipped, ctx.machine.total_cycles, steps_executed
        );
    }

    let uart_text = {
        let bytes = ctx.uart_tx.lock().map(|g| g.clone()).unwrap_or_default();
        String::from_utf8_lossy(&bytes).to_string()
    };

    // Finalize main-stack report before assertion evaluation so
    // `resource_budget` can compare against high-water / footprint.
    // Snapshot bus access counts before paint scan / memory assertions pollute
    // the run-lifetime counters.
    let (memory_reads, memory_writes, peripheral_accesses) = ctx.machine.bus.access_counts();
    let top_pcs = resource_report::top_pc_samples(&pc_hist, resource_report::PC_SAMPLE_TOP_N);
    let pc_samples = resource_report::resolve_pc_sample_symbols(&top_pcs, ctx.firmware_path);
    let execution_metrics = artifacts::ExecutionMetrics {
        cycles: ctx.metrics.get_cycles(),
        instructions: ctx.metrics.get_instructions(),
        steps_executed,
        memory_reads,
        memory_writes,
        peripheral_accesses,
        exceptions: exception_count,
        pc_samples,
    };

    let memory = if let Some(session) = paint_session {
        let final_sp = resource_report::arm_sp(&ctx.machine.cpu);
        resource_report::finalize_paint_report(&ctx.machine.bus, final_sp, session)
    } else {
        memory_pre
    };

    let mut assertion_results = Vec::new();
    let mut all_passed = true;
    let mut expected_stop_reason_matched = false;

    for (assertion_index, assertion) in ctx.assertions.iter().enumerate() {
        let (passed, evidence) = match assertion {
            TestAssertion::UartContains(_)
            | TestAssertion::UartRegex(_)
            | TestAssertion::UartOrdered(_)
            | TestAssertion::MotorState(_)
            | TestAssertion::MqttFabric(_) => (
                assertion_currently_passes(assertion, &uart_text, ctx.machine),
                None,
            ),
            TestAssertion::MotorSpeedReached(_) => (
                assertion_latched[assertion_index]
                    || assertion_currently_passes(assertion, &uart_text, ctx.machine),
                None,
            ),
            TestAssertion::ShutdownLatency(a) => {
                let passed = shutdown_latency_passes(
                    &a.shutdown_latency,
                    &stimulus_cycles,
                    &uart_milestone_cycles,
                );
                let evidence = shutdown_latency_cycles(
                    &a.shutdown_latency,
                    &stimulus_cycles,
                    &uart_milestone_cycles,
                )
                .map(|(stimulus_cycle, token_cycle, latency_cycles)| {
                    AssertionEvidence::ShutdownLatency {
                        stimulus_cycle,
                        token_cycle,
                        latency_cycles,
                        configured_max_cycles: a.shutdown_latency.max_cycles,
                    }
                });
                (passed, evidence)
            }
            TestAssertion::ExpectedStopReason(a) => (a.expected_stop_reason == stop_reason, None),
            // Passes only if the FIRMWARE ended the run with exactly this code.
            // A timeout, halt or fault leaves `firmware_exit_code` None, so a
            // run that never reached its own success path fails rather than
            // passing by silence.
            TestAssertion::FirmwareExit(a) => (firmware_exit_code == Some(a.firmware_exit), None),
            TestAssertion::MemoryValue(a) => {
                // `size` is the value width. Accept either bytes (1/2/4) or
                // bits (8/16/32) — both name the same u8/u16/u32 reads — so a
                // natural "4 bytes" guess for a u32 RAM word works as well as
                // the historical bit-width form. Defaults to a 32-bit (u32) word.
                let size = a.memory_value.size.unwrap_or(32);
                let result = match size {
                    1 | 8 => ctx
                        .machine
                        .bus
                        .read_u8(a.memory_value.address)
                        .map(|v| v as u32),
                    2 | 16 => ctx
                        .machine
                        .bus
                        .read_u16(a.memory_value.address)
                        .map(|v| v as u32),
                    4 | 32 => ctx.machine.bus.read_u32(a.memory_value.address),
                    _ => {
                        error!(
                            "Unsupported memory assertion size: {} — use 1/2/4 (bytes) or 8/16/32 (bits)",
                            size
                        );
                        Err(labwired_core::SimulationError::Other("Invalid size".into()))
                    }
                };

                let passed = match result {
                    Ok(val) => {
                        let mask = a.memory_value.mask.unwrap_or(0xFFFFFFFF) as u32;
                        let expected = a.memory_value.expected_value as u32;
                        let matched = (val & mask) == (expected & mask);
                        if !matched {
                            error!(
                                "Memory assertion failed at {:#x} (size {}): expected {:#x}, got {:#x} (mask {:#x})",
                                a.memory_value.address, size, expected, val, mask
                            );
                        }
                        matched
                    }
                    Err(e) => {
                        error!(
                            "Memory assertion failed to read address {:#x} (size {}): {}",
                            a.memory_value.address, size, e
                        );
                        false
                    }
                };
                (passed, None)
            }
            TestAssertion::UdsTester(a) => {
                let passed =
                    match evaluate_uds_tester(&ctx.machine.bus.can_uds_testers, &a.uds_tester) {
                        Ok(()) => true,
                        Err(msg) => {
                            error!("Assertion failed: {}", msg);
                            false
                        }
                    };
                (passed, None)
            }
            // The measurement itself carries the diagnosis (which region, how
            // much ink, what was required), so it is logged rather than
            // reduced to a bare `false`.
            TestAssertion::DisplayRegion(a) => {
                let passed = match evaluate_display_region(&ctx.machine.bus, &a.display_region) {
                    Ok(()) => true,
                    Err(msg) => {
                        error!("Assertion failed: {}", msg);
                        false
                    }
                };
                (passed, None)
            }
            TestAssertion::ResourceBudget(a) => {
                evaluate_resource_budget(&a.resource_budget, footprint.as_ref(), Some(&memory))
            }
        };

        if matches!(assertion, TestAssertion::ExpectedStopReason(_)) && passed {
            expected_stop_reason_matched = true;
        }

        if !passed {
            all_passed = false;
            error!(
                "Assertion failed: {:?} (captured len={})",
                assertion,
                uart_text.len()
            );
        }

        assertion_results.push(AssertionResult {
            assertion: assertion.clone(),
            passed,
            evidence,
        });
    }

    let stop_requires_assertion = matches!(
        stop_reason,
        StopReason::WallTime | StopReason::MaxUartBytes | StopReason::NoProgress
    );

    // Any `after_cycles` stimulus whose threshold the run never reached also
    // proved nothing about that input, so it is recorded rather than dropped.
    // Unlike a rejection this is NOT fatal: a run can legitimately stop early
    // (`stop_when_assertions_pass`) with a later rung of a stimulus ladder
    // unfired, and failing those would flip existing green runs red on a pacing
    // judgement call. It is reported so the reader can see it.
    {
        let end_cycle = ctx.machine.total_cycles;
        for (s, fired) in &pending_stimuli {
            if *fired {
                continue;
            }
            let threshold = match s.trigger {
                labwired_config::FaultTrigger::AfterCycles { cycles } => cycles,
                _ => continue,
            };
            let outcome = StimulusOutcome::new(
                s,
                artifacts::STIMULUS_NOT_REACHED,
                end_cycle,
                Some(format!(
                    "never fired: the run ended at cycle {end_cycle}, before the after_cycles \
                     threshold {threshold}"
                )),
            );
            error!(
                "stimulus '{}' = {} never fired: the run ended at cycle {end_cycle}, before its \
                 after_cycles threshold {threshold}",
                outcome.channel, outcome.value
            );
            stimulus_outcomes.push(outcome);
        }
    }

    // A stimulus the engine REFUSED never reached the device, so nothing the
    // run observed can be attributed to it — a "pass" here would be a run that
    // proved nothing, which is the single most expensive failure mode in this
    // codebase. It is therefore an invalid run, not a firmware verdict:
    // `status: "error"` + `EXIT_CONFIG_ERROR`, exactly like the `uart_injection`
    // peripheral-not-found gate above, which is the same class of failure
    // (declared input never delivered) and already hard-fails.
    let stimuli_rejected = stimulus_outcomes.iter().filter(|o| o.is_rejected()).count();
    if stimuli_rejected > 0 {
        error!(
            "{stimuli_rejected} stimulus/ctx.stimuli could not be applied; the run is invalid \
             (nothing it observed can be attributed to them)"
        );
    }

    // `rejected` dominates the other verdicts on purpose: a "fail" produced by a
    // run whose inputs were never delivered is not a trustworthy fail either.
    // A firmware that declared its own failure fails the run, whether or not
    // the script asserted anything. Without this a run with no assertions would
    // report `status: "pass"` for firmware that explicitly said `EXIT 5` — the
    // proved-nothing failure mode again, and the worse for being self-inflicted:
    // the run has an unambiguous verdict from the firmware itself and would be
    // ignoring it. `None` (a bare `STOP`, or any non-simctl stop) is not a
    // failure claim and does not trigger this.
    let firmware_declared_failure = firmware_exit_code.is_some_and(|code| code != 0);
    if firmware_declared_failure {
        error!(
            "firmware ended the run with a non-zero exit code ({}); the run fails",
            firmware_exit_code.unwrap_or_default()
        );
    }

    // Finalise runtime-observed fault outcomes (e.g. missing_clock fires only
    // when the firmware actually accessed the unclocked peripheral) and enforce
    // the require_fault_fired gate: a fault that never took effect makes the run
    // invalid, not a firmware pass.
    //
    // This must happen BEFORE the verdict, not after it. It used to sit thirty
    // lines below, which is precisely why `fault_gate_failed` could only reach
    // the exit code and never the `status` — the artifact certified a pass for
    // a run the exit code called invalid. See `crate::verdict`.
    labwired_cli::faults::finalize_fault_evidence(
        &ctx.machine.bus,
        ctx.faults,
        &mut ctx.fault_evidence,
    );
    let fault_gate_failed = ctx.require_fault_fired && ctx.fault_evidence.iter().any(|e| !e.fired);
    if fault_gate_failed {
        let n = ctx.fault_evidence.iter().filter(|e| !e.fired).count();
        error!("ctx.require_fault_fired: {n} fault(s) did not fire; run is invalid");
    }

    // THE verdict. One decision, from which both `status` and the exit code are
    // read below — they cannot disagree because there is nothing left to
    // disagree with. Do not reintroduce a second chain here.
    let verdict = crate::verdict::RunFacts {
        stimuli_rejected: stimuli_rejected > 0,
        firmware_declared_failure,
        assertions_failed: !all_passed,
        fault_gate_failed,
        unexpected_safety_stop: stop_requires_assertion && !expected_stop_reason_matched,
        unrescued_runtime_error: sim_error_happened && !expected_stop_reason_matched,
    }
    .verdict();

    let duration = start.elapsed();
    let uart_bytes = ctx.uart_tx.lock().map(|g| g.len() as u64).unwrap_or(0);
    let stop_reason_details = crate::report::build_stop_reason_details(
        &stop_reason,
        ctx.resolved_limits,
        steps_executed,
        // The clock `max_cycles` is checked against, so a `max_cycles` stop
        // reports the observation that crossed it.
        ctx.machine.total_cycles,
        uart_bytes,
        stuck_counter,
        duration,
        0, // vcd_bytes - will be updated below
    );
    // Final-state universal inspect block (summary mode: decoded registers +
    // artifact metadata, framebuffer bytes omitted/hashed). This is the
    // agent-facing oracle payload — after a run the caller sees the decoded
    // final register state and which artifacts exist.
    let inspect_block = ctx.machine.inspect(
        None,
        &labwired_core::inspect::InspectOpts {
            include_bytes: false,
            peripheral: None,
        },
    );

    // Drain the deterministic logic-analyzer edge capture for THIS run and shape
    // it into the shared per-channel series form. Reading from cursor 0 returns
    // every retained edge; `dropped` (surfaced in the block) is non-zero only if
    // the 64k ring overflowed, which the oracle treats as fail-loud. This is the
    // SAME `logic_read_edges` drain the wasm `read_logic_edges` accessor uses, so
    // the CLI `result.json` edges and the browser edges are edge-for-edge equal.
    let logic_edges = if logic_capture_armed {
        let now_cycle = ctx.machine.logic_now_cycle();
        let batch = ctx.machine.logic_read_edges(0);
        Some(labwired_core::logic_capture::build_logic_edges_result(
            &logic_watch_meta,
            &batch,
            now_cycle,
        ))
    } else {
        None
    };

    export_analog_trace_if_requested(&ctx.args.analog_trace, ctx.machine);

    // ── THE VERDICT ──────────────────────────────────────────────────────────
    //
    // `labwired test` is the deterministic gate, and until now a PASSING run
    // said nothing at all about what it had verified: failures went out through
    // `error!`, passes were silent, and the only machine-readable answer lived
    // in `result.json` / JUnit. A human running the gate in a terminal saw the
    // firmware's own UART output and had to infer the verdict from `$?`.
    //
    // One line, on STDERR. That is deliberate and it is what makes this safe to
    // print unconditionally: firmware UART echo and the `--json` agent payload
    // both go to stdout, so a human-facing line on stderr can never corrupt a
    // piped capture or a JSON parse. No new parameter threaded through this
    // already twenty-argument signature to decide whether to speak.
    {
        let checked = assertion_results.len();
        let passed = assertion_results.iter().filter(|a| a.passed).count();
        let label = verdict.banner_label();
        // The SCRIPT, not the system manifest: nearly every board ships its
        // manifest as `system.yaml`, so naming that would print the same
        // uninformative "system" for every board in the repo. The script stem
        // is what the caller actually typed and what a CI log needs to
        // identify. Firmware stem is the fallback for a scriptless run.
        let subject = ctx
            .args
            .script
            .file_stem()
            .or_else(|| ctx.firmware_path.file_stem())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "run".to_string());
        eprintln!(
            "{label}  {passed}/{checked} checks · {subject} · {steps_executed} steps · {:.2}s",
            duration.as_secs_f64()
        );
    }

    write_outputs(
        ctx.args,
        verdict,
        steps_executed,
        ctx.metrics,
        stop_reason.clone(),
        stop_reason_details,
        firmware_exit_code,
        ctx.resolved_limits.clone(),
        assertion_results,
        ctx.firmware_bytes,
        ctx.uart_tx,
        &ctx.machine.cpu,
        ctx.firmware_path,
        ctx.system_path,
        duration,
        &trace_observer,
        &coverage_observer,
        &ctx.fault_evidence,
        Some(inspect_block),
        logic_edges,
        stimulus_outcomes,
        footprint,
        Some(memory),
        Some(execution_metrics),
    );

    // The same `verdict` the artifact above was written from. Not a second
    // chain — that is the whole point of `crate::verdict`.
    verdict.exit_code()
}
