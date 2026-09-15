// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Cortex-M JIT vs interpreter on the nRF timer / UART / WFI surfaces the
//! interpreter already owns. JIT must match; WFI stays interpreter-owned.

#![cfg(all(feature = "jit", feature = "event-scheduler"))]

use labwired_core::bus::{SystemBus, RECOMMENDED_TICK_INTERVAL};
use labwired_core::cpu::jit_framework::cortex_m::snapshot_state;
use labwired_core::cpu::CortexM;
use labwired_core::{Bus, DebugControl, Machine};

fn h(bytes: &mut Vec<u8>, half: u16) {
    bytes.extend_from_slice(&half.to_le_bytes());
}

fn wfi_idle_machine(jit: bool) -> Machine<CortexM> {
    let mut bus = SystemBus::new();
    bus.write_u16(0x0, 0xBF30).unwrap(); // WFI
    bus.write_u16(0x2, 0xE7FD).unwrap(); // B -> 0x0
    let mut cpu = CortexM::new();
    cpu.pc = 0x0;
    cpu.sp = 0x2000_1000;
    let mut machine = Machine::new(cpu, bus);
    machine.config.idle_fast_forward_enabled = true;
    machine.config.cortex_m_jit_enabled = jit;
    machine.bus.config.cortex_m_jit_enabled = jit;
    machine.bus.legacy_walk_disabled = true;
    machine
}

#[test]
fn wfi_idle_fast_forward_matches_with_jit_on() {
    let mut off = wfi_idle_machine(false);
    let mut on = wfi_idle_machine(true);
    off.run(Some(64)).unwrap();
    on.run(Some(64)).unwrap();
    assert_eq!(
        off.total_cycles, on.total_cycles,
        "WFI idle-ff must not change total_cycles when the JIT is on"
    );
    assert_eq!(off.cpu.pc, on.cpu.pc);
    assert_eq!(
        snapshot_state(&off.cpu),
        snapshot_state(&on.cpu),
        "WFI path is interpreter-owned; JIT on/off must match"
    );
}

fn alu_spin_machine(jit: bool, tick: u32) -> Machine<CortexM> {
    let mut prog = Vec::new();
    // movs r0, #0 ; then a long adds body + b back. 20 ALU + branch.
    h(&mut prog, 0x2000); // movs r0, #0
    for _ in 0..20 {
        h(&mut prog, 0x3001); // adds r0, #1
    }
    // branch from pc=42 back to pc=2 (skip movs)
    // offset = 2 - (42+4) = -44; imm11 = -22 = 0x7EA
    h(&mut prog, 0xE7EA);
    let mut bus = SystemBus::new();
    let n = prog.len();
    bus.flash.data[..n].copy_from_slice(&prog);
    let mut cpu = CortexM::new();
    cpu.pc = 0;
    cpu.sp = 0x2000_1000;
    cpu.xpsr = 0x0100_0000;
    let mut machine = Machine::new(cpu, bus);
    machine.config.cortex_m_jit_enabled = jit;
    machine.bus.config.cortex_m_jit_enabled = jit;
    machine.config.peripheral_tick_interval = tick;
    machine.bus.config.peripheral_tick_interval = tick;
    machine.bus.legacy_walk_disabled = true;
    machine
}

#[test]
fn alu_spin_matches_at_tick_512() {
    let mut off = alu_spin_machine(false, RECOMMENDED_TICK_INTERVAL);
    let mut on = alu_spin_machine(true, RECOMMENDED_TICK_INTERVAL);
    const STEPS: u32 = 8_000;
    off.run(Some(STEPS)).unwrap();
    on.run(Some(STEPS)).unwrap();
    assert_eq!(off.total_cycles, on.total_cycles);
    assert_eq!(off.cpu.pc, on.cpu.pc);
    assert_eq!(off.cpu.r0, on.cpu.r0);
    assert_eq!(
        off.cpu.xpsr & 0xF000_0000,
        on.cpu.xpsr & 0xF000_0000,
        "NZCV"
    );
    if let Some(stats) = on.cpu.jit_stats() {
        assert!(
            stats.block_runs > 0,
            "tick-512 ALU spin must actually run compiled blocks; stats={stats:?}"
        );
    } else {
        panic!("JIT engine was never created");
    }
}

#[test]
fn uart_mmio_store_exits_and_matches() {
    // STR r0, [r1, #0] to UART data (MMIO) then branch back. The store must
    // side-exit to the interpreter so UART TX side effects stay identical.
    let mut prog = Vec::new();
    h(&mut prog, 0x2001); // movs r0, #1
    h(&mut prog, 0x6008); // str r0, [r1, #0]
    h(&mut prog, 0xE7FC); // b to str (pc=2)
    let build = |jit: bool| {
        let mut bus = SystemBus::new();
        let n = prog.len();
        bus.flash.data[..n].copy_from_slice(&prog);
        let mut cpu = CortexM::new();
        cpu.pc = 0;
        cpu.r1 = 0x4000_C000; // default SystemBus uart1
        cpu.sp = 0x2000_1000;
        cpu.xpsr = 0x0100_0000;
        let mut machine = Machine::new(cpu, bus);
        machine.config.cortex_m_jit_enabled = jit;
        machine.bus.config.cortex_m_jit_enabled = jit;
        machine
    };
    let mut off = build(false);
    let mut on = build(true);
    off.run(Some(64)).unwrap();
    on.run(Some(64)).unwrap();
    assert_eq!(off.cpu.pc, on.cpu.pc);
    assert_eq!(off.cpu.r0, on.cpu.r0);
    assert_eq!(off.total_cycles, on.total_cycles);
}

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{AdvanceRequest, BreakpointPolicy, Cpu};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load_system(chip_rel: &str, system_rel: &str) -> (ChipDescriptor, SystemManifest) {
    let root = repo_root();
    let chip = ChipDescriptor::from_file(root.join(chip_rel)).expect("chip yaml");
    let sys_path = root.join(system_rel);
    let mut manifest = SystemManifest::from_file(&sys_path).expect("system yaml");
    manifest.chip = sys_path
        .parent()
        .unwrap()
        .join(&manifest.chip)
        .to_string_lossy()
        .into_owned();
    (chip, manifest)
}

fn nrf_uart_smoke(jit: bool) -> (Machine<CortexM>, Arc<Mutex<Vec<u8>>>) {
    let (chip, manifest) = load_system(
        "configs/chips/nrf52832.yaml",
        "configs/systems/nrf52-dk.yaml",
    );
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("nrf52 bus");
    let sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    let code = [
        0x02u8, 0x48, // ldr r0, [pc, #8]
        0x4F, 0x21, // movs r1, #79 'O'
        0x01, 0x60, // str r1, [r0]
        0x4B, 0x21, // movs r1, #75 'K'
        0x01, 0x60, // str r1, [r0]
        0xFE, 0xE7, // b .
        0x1C, 0x25, 0x00, 0x40, // 0x4000251C UART0 TXD
    ];
    for (i, b) in code.iter().enumerate() {
        assert!(bus.flash.write_u8(i as u64, *b));
    }
    let mut cpu = CortexM::new();
    cpu.set_pc(0);
    let mut machine = Machine::new(cpu, bus);
    machine.config.cortex_m_jit_enabled = jit;
    machine.bus.config.cortex_m_jit_enabled = jit;
    machine.config.peripheral_tick_interval = RECOMMENDED_TICK_INTERVAL;
    machine.bus.config.peripheral_tick_interval = RECOMMENDED_TICK_INTERVAL;
    (machine, sink)
}

#[test]
fn nrf52_uart_txd_matches_with_jit() {
    let (mut off, sink_off) = nrf_uart_smoke(false);
    let (mut on, sink_on) = nrf_uart_smoke(true);
    off.run(Some(32)).unwrap();
    on.run(Some(32)).unwrap();
    let a = sink_off.lock().unwrap().clone();
    let b = sink_on.lock().unwrap().clone();
    assert_eq!(a, b, "UART TX bytes JIT vs interpreter");
    assert_eq!(a.last().copied(), Some(b'K'));
    assert_eq!(off.total_cycles, on.total_cycles);
    assert_eq!(snapshot_state(&off.cpu), snapshot_state(&on.cpu));
}

fn nrf52840_timer_spin(jit: bool) -> Machine<CortexM> {
    let (chip, mut manifest) = load_system(
        "configs/chips/nrf52840.yaml",
        "configs/systems/nrf52840-dk.yaml",
    );
    manifest.walk_deleted = None;
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("nrf52840 bus");
    let _ = configure_cortex_m(&mut bus);
    let mut prog = Vec::new();
    h(&mut prog, 0x2000);
    for _ in 0..20 {
        h(&mut prog, 0x3001);
    }
    h(&mut prog, 0xE7EA);
    let n = prog.len();
    bus.flash.data[..n].copy_from_slice(&prog);
    let mut cpu = CortexM::new();
    cpu.pc = 0;
    cpu.sp = 0x2000_1000;
    cpu.xpsr = 0x0100_0000;
    let mut machine = Machine::new(cpu, bus);
    machine.config.cortex_m_jit_enabled = jit;
    machine.bus.config.cortex_m_jit_enabled = jit;
    machine.config.peripheral_tick_interval = RECOMMENDED_TICK_INTERVAL;
    machine.bus.config.peripheral_tick_interval = RECOMMENDED_TICK_INTERVAL;
    machine.config.idle_fast_forward_enabled = false;

    const TIMER0: u64 = 0x4000_8000;
    machine.bus.write_u32(TIMER0 + 0x508, 3).unwrap(); // BITMODE
    machine.bus.write_u32(TIMER0 + 0x510, 0).unwrap(); // PRESCALER
    machine.bus.write_u32(TIMER0 + 0x540, 8).unwrap(); // CC0
    machine.bus.write_u32(TIMER0 + 0x304, 1 << 16).unwrap(); // INTENSET
    machine.bus.write_u32(TIMER0 + 0x00C, 1).unwrap(); // CLEAR
    machine.bus.write_u32(TIMER0, 1).unwrap(); // START
    machine
}

#[test]
fn nrf52840_timer0_compare_cycle_matches_with_jit() {
    const EVENTS_COMPARE0: u64 = 0x4000_8000 + 0x140;
    let mut off = nrf52840_timer_spin(false);
    let mut on = nrf52840_timer_spin(true);
    let mut fire_off = None;
    let mut fire_on = None;
    const BUDGET: u64 = 4_096;
    while off.total_cycles < BUDGET || on.total_cycles < BUDGET {
        if fire_off.is_none() && off.total_cycles < BUDGET {
            off.advance(AdvanceRequest::run(Some(512)).with_breakpoints(BreakpointPolicy::Ignore))
                .unwrap();
            if off.bus.read_u32(EVENTS_COMPARE0).unwrap_or(0) != 0 {
                fire_off = Some(off.total_cycles);
            }
        }
        if fire_on.is_none() && on.total_cycles < BUDGET {
            on.advance(AdvanceRequest::run(Some(512)).with_breakpoints(BreakpointPolicy::Ignore))
                .unwrap();
            if on.bus.read_u32(EVENTS_COMPARE0).unwrap_or(0) != 0 {
                fire_on = Some(on.total_cycles);
            }
        }
        if fire_off.is_some() && fire_on.is_some() {
            break;
        }
    }
    assert_eq!(
        fire_off, fire_on,
        "TIMER0 COMPARE[0] fire cycle JIT vs interpreter"
    );
    assert!(fire_off.is_some(), "TIMER0 never fired");
}

fn zephyr_hello(jit: bool) -> (Machine<CortexM>, Arc<Mutex<Vec<u8>>>) {
    let (chip, mut manifest) = load_system(
        "configs/chips/nrf52840.yaml",
        "validation/zephyr-matrix/systems/nrf52840.yaml",
    );
    manifest.walk_deleted = None;
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("zephyr nrf52840 bus");
    let sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let elf = repo_root().join("tests/fixtures/nrf52840-zephyr-l0-hello.elf");
    let image = labwired_loader::load_elf(&elf).expect("load hello elf");
    machine.load_firmware(&image).expect("load firmware");
    machine.config.cortex_m_jit_enabled = jit;
    machine.bus.config.cortex_m_jit_enabled = jit;
    machine.config.peripheral_tick_interval = RECOMMENDED_TICK_INTERVAL;
    machine.bus.config.peripheral_tick_interval = RECOMMENDED_TICK_INTERVAL;
    machine.config.idle_fast_forward_enabled = false;
    (machine, sink)
}

fn has_marker(sink: &Arc<Mutex<Vec<u8>>>, marker: &[u8]) -> bool {
    sink.lock()
        .unwrap()
        .windows(marker.len())
        .any(|w| w == marker)
}

fn catch_up(machine: &mut Machine<CortexM>, target_insns: u64) {
    while machine.step_profile().cpu_instructions < target_insns {
        let remain = (target_insns - machine.step_profile().cpu_instructions).min(512);
        machine.run(Some(remain as u32)).expect("catch-up");
    }
}

fn snapshot_diff(off: &[u32], on: &[u32]) -> String {
    const NAMES: [&str; 21] = [
        "r0",
        "r1",
        "r2",
        "r3",
        "r4",
        "r5",
        "r6",
        "r7",
        "r8",
        "r9",
        "r10",
        "r11",
        "r12",
        "sp",
        "lr",
        "pc",
        "xpsr",
        "primask",
        "faultmask",
        "basepri",
        "it_state",
    ];
    let mut out = String::new();
    let n = off.len().min(on.len());
    for i in 0..n {
        if off[i] != on[i] {
            let name = NAMES.get(i).copied().unwrap_or("?");
            out.push_str(&format!(" {name}[{i}]: off={:#x} on={:#x}", off[i], on[i]));
        }
    }
    if off.len() != on.len() {
        out.push_str(&format!(" len off={} on={}", off.len(), on.len()));
    }
    if out.is_empty() {
        out.push_str(" (equal)");
    }
    out
}

#[test]
fn nrf52840_zephyr_hello_uart_and_cycles_match_at_tick_512() {
    let (mut off, sink_off) = zephyr_hello(false);
    let (mut on, sink_on) = zephyr_hello(true);
    const MARKER: &[u8] = b"LW_Z0_OK";
    const MAX: u64 = 80_000;
    let mut chunks = 0u32;
    while off.step_profile().cpu_instructions < MAX
        && on.step_profile().cpu_instructions < MAX
        && (!has_marker(&sink_off, MARKER) || !has_marker(&sink_on, MARKER))
    {
        off.run(Some(64)).expect("interp chunk");
        on.run(Some(64)).expect("jit chunk");
        let target = off
            .step_profile()
            .cpu_instructions
            .max(on.step_profile().cpu_instructions);
        catch_up(&mut off, target);
        catch_up(&mut on, target);
        chunks += 1;
        let snap_off = snapshot_state(&off.cpu);
        let snap_on = snapshot_state(&on.cpu);
        let uart_off = sink_off.lock().unwrap().clone();
        let uart_on = sink_on.lock().unwrap().clone();
        if snap_off != snap_on || uart_off != uart_on {
            panic!(
                "diverged after {chunks} chunks insns={} cycles off={} on={} off_pc={:#x} on_pc={:#x} regs={} off_uart={:?} on_uart={:?} jit={:?}",
                off.step_profile().cpu_instructions,
                off.total_cycles,
                on.total_cycles,
                off.cpu.pc,
                on.cpu.pc,
                snapshot_diff(&snap_off, &snap_on),
                String::from_utf8_lossy(&uart_off),
                String::from_utf8_lossy(&uart_on),
                on.cpu.jit_stats(),
            );
        }
    }
    assert!(
        has_marker(&sink_off, MARKER),
        "interpreter never printed LW_Z0_OK: {}",
        String::from_utf8_lossy(&sink_off.lock().unwrap())
    );
    assert!(
        has_marker(&sink_on, MARKER),
        "JIT never printed LW_Z0_OK: {}",
        String::from_utf8_lossy(&sink_on.lock().unwrap())
    );
    let target = off
        .step_profile()
        .cpu_instructions
        .max(on.step_profile().cpu_instructions);
    catch_up(&mut off, target);
    catch_up(&mut on, target);
    let uart_off = sink_off.lock().unwrap().clone();
    let uart_on = sink_on.lock().unwrap().clone();
    eprintln!(
        "hello fidelity: off insns={} cycles={} pc={:#x} uart_len={}\n            on  insns={} cycles={} pc={:#x} uart_len={} jit={:?}",
        off.step_profile().cpu_instructions,
        off.total_cycles,
        off.cpu.pc,
        uart_off.len(),
        on.step_profile().cpu_instructions,
        on.total_cycles,
        on.cpu.pc,
        uart_on.len(),
        on.cpu.jit_stats(),
    );
    assert_eq!(
        uart_off,
        uart_on,
        "Zephyr hello UART JIT vs interpreter\n off={:?}\n on={:?}",
        String::from_utf8_lossy(&uart_off),
        String::from_utf8_lossy(&uart_on)
    );
    assert_eq!(
        off.step_profile().cpu_instructions,
        on.step_profile().cpu_instructions,
        "instruction count"
    );
    assert_eq!(
        off.total_cycles, on.total_cycles,
        "total_cycles at tick 512"
    );
    assert_eq!(off.cpu.pc, on.cpu.pc, "pc");
    assert_eq!(
        snapshot_state(&off.cpu),
        snapshot_state(&on.cpu),
        "arch state after hello"
    );
    let stats = on.cpu.jit_stats().expect("JIT engine must exist");
    assert!(
        stats.block_runs > 0,
        "Zephyr hello compiled no blocks: {stats:?}"
    );
    assert!(
        stats.block_instrs > 0,
        "Zephyr hello retired no compiled insns: {stats:?}"
    );
}

#[test]
fn nrf52840_zephyr_hello_matches_at_min_block_4() {
    let (mut off, sink_off) = zephyr_hello(false);
    let (mut on, sink_on) = zephyr_hello(true);
    on.config.cortex_m_jit_min_block_instrs = 4;
    on.bus.config.cortex_m_jit_min_block_instrs = 4;
    const MARKER: &[u8] = b"LW_Z0_OK";
    const MAX: u64 = 80_000;
    let mut chunks = 0u32;
    while off.step_profile().cpu_instructions < MAX
        && on.step_profile().cpu_instructions < MAX
        && (!has_marker(&sink_off, MARKER) || !has_marker(&sink_on, MARKER))
    {
        off.run(Some(64)).expect("interp chunk");
        on.run(Some(64)).expect("jit chunk");
        let target = off
            .step_profile()
            .cpu_instructions
            .max(on.step_profile().cpu_instructions);
        catch_up(&mut off, target);
        catch_up(&mut on, target);
        chunks += 1;
        let snap_off = snapshot_state(&off.cpu);
        let snap_on = snapshot_state(&on.cpu);
        if snap_off != snap_on {
            panic!(
                "min4 diverged after {chunks} chunks insns={} {}",
                off.step_profile().cpu_instructions,
                snapshot_diff(&snap_off, &snap_on)
            );
        }
    }
    assert!(has_marker(&sink_off, MARKER) && has_marker(&sink_on, MARKER));
    let stats = on.cpu.jit_stats().expect("JIT");
    eprintln!(
        "hello min4: insns={} pc={:#x} uart={} jit={:?}",
        on.step_profile().cpu_instructions,
        on.cpu.pc,
        sink_on.lock().unwrap().len(),
        stats
    );
    assert_eq!(off.cpu.pc, on.cpu.pc);
    assert_eq!(snapshot_state(&off.cpu), snapshot_state(&on.cpu));
    assert!(stats.block_instrs >= 4, "min4 compiled nothing: {stats:?}");
    assert_eq!(stats.ram_bytes_synced, 0, "hello min4 memcpy: {stats:?}");
}

fn plant_systick_handler(machine: &mut Machine<CortexM>) {
    const HANDLER: u32 = 0x80;
    machine
        .bus
        .write_u32(15 * 4, HANDLER | 1)
        .expect("SysTick vector");
    machine
        .bus
        .write_u16(HANDLER as u64, 0xE7FE)
        .expect("SysTick handler b .");
}

#[test]
fn takeable_irq_at_batch_start_is_taken_before_compiled_block() {
    let mut off = alu_spin_machine(false, RECOMMENDED_TICK_INTERVAL);
    let mut on = alu_spin_machine(true, RECOMMENDED_TICK_INTERVAL);
    plant_systick_handler(&mut off);
    plant_systick_handler(&mut on);

    const MAX_HEAT: u32 = 16_000;
    let mut heated = 0u32;
    while on.cpu.jit_stats().map(|s| s.block_runs).unwrap_or(0) == 0 {
        assert!(
            heated < MAX_HEAT,
            "ALU spin never ran a compiled block: {:?}",
            on.cpu.jit_stats()
        );
        on.run(Some(64)).expect("heat jit");
        heated += 64;
    }
    catch_up(&mut off, on.step_profile().cpu_instructions);
    // The compiled spin lives at pc=2. Align there so the next batch is
    // Lookup::Ready and would run the whole user block if dispatch is wrong.
    const LOOP_PC: u32 = 2;
    while on.cpu.pc != LOOP_PC {
        on.run(Some(1)).expect("align jit to loop entry");
    }
    catch_up(&mut off, on.step_profile().cpu_instructions);
    assert_eq!(
        off.cpu.pc, LOOP_PC,
        "interpreter twin must sit on the hot entry"
    );
    assert_eq!(
        snapshot_state(&off.cpu),
        snapshot_state(&on.cpu),
        "pre-IRQ state must match after heat"
    );
    let r0_before = on.cpu.r0;

    on.cpu.set_exception_pending(15);
    off.cpu.set_exception_pending(15);
    on.run(Some(64)).expect("jit take irq");
    off.run(Some(64)).expect("interp take irq");

    assert_eq!(
        on.cpu.active_exception, 15,
        "JIT must take SysTick (15), not skip it; active={} pc={:#x}",
        on.cpu.active_exception, on.cpu.pc
    );
    assert_eq!(
        off.cpu.active_exception,
        on.cpu.active_exception,
        "SysTick must be taken on both; off={} on={} r0 off={:#x} on={:#x} pc off={:#x} on={:#x}",
        off.cpu.active_exception,
        on.cpu.active_exception,
        off.cpu.r0,
        on.cpu.r0,
        off.cpu.pc,
        on.cpu.pc
    );
    assert_eq!(
        snapshot_state(&off.cpu),
        snapshot_state(&on.cpu),
        "JIT must take SysTick at batch start, not after a compiled user block; {}",
        snapshot_diff(&snapshot_state(&off.cpu), &snapshot_state(&on.cpu))
    );
    assert_eq!(
        on.cpu.r0, r0_before,
        "JIT must not retire extra user ALU before taking SysTick (r0 {} -> {})",
        r0_before, on.cpu.r0
    );
}
