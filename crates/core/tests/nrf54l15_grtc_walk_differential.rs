// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Differential gate for the nRF54L GRTC scheduler migration (idle
//! fast-forward), in the `systick_walk_differential` style: the SAME hand-built
//! Cortex-M33 machine and hand-assembled Thumb firmware run twice — once with
//! the GRTC pinned onto the per-cycle walk (`force_legacy_walk`, the reference)
//! and once scheduler-driven — and every observable is compared.
//!
//! The GRTC is exactly the kind of peripheral this proof exists for: a
//! free-running SYSCOUNTER that firmware POLLS (the derived-read path) plus an
//! absolute compare that delivers the kernel tick as an IRQ (the scheduled-
//! event path). Both must be byte-identical to the legacy walk.
//!
//! 1. `grtc_compare_irq_firmware_is_byte_identical_at_interval_1` — firmware
//!    that starts the SYSCOUNTER, arms a periodic compare (the ISR re-arms via
//!    CCADD, exactly as nrfx does), enables the group-0 GRTC line, and polls
//!    SYSCOUNTERL from its main loop. Probed after EVERY instruction: total
//!    cycles, PC, all 16 core registers (incl. r4 = the raw SYSCOUNTER poll)
//!    and both RAM counters must be byte-identical — pinning the IRQ delivery
//!    cycles, the ISR execution count, and the derived counter exactly.
//!
//! 2. `grtc_isr_count_is_exact_at_interval_64` — the scheduler lane at tick
//!    interval 64 vs the walk-on interval-1 golden reference: IRQ delivery
//!    quantises to the batch grid (documented, ≤ one interval), but the ISR
//!    execution count over a fixed instruction window is EXACT (absolute
//!    compare deadlines, no cumulative drift).
//!
//! 3. `wfi_sleep_wakes_from_scheduled_grtc_compare` — the canonical tickless
//!    idle pattern (`wfi` in the loop): with idle fast-forward enabled the
//!    machine must skip straight to the SCHEDULED GRTC compare deadline (the FF
//!    budget clamps to `next_event_deadline`) and wake with byte-identical
//!    architectural state vs the FF-off run, retiring far fewer instructions.

#![cfg(feature = "event-scheduler")]

use labwired_core::bus::SystemBus;
use labwired_core::cpu::CortexM;
use labwired_core::peripherals::nrf54l::grtc::Nrf54lGrtc;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::Bus;
use labwired_core::{DebugControl, Machine};

// GRTC placed in the SoC peripheral window, away from the SCB/NVIC/SysTick
// block at 0xE000_Exxx.
const GRTC_BASE: u32 = 0x5000_0000;
const ISR_COUNT_ADDR: u64 = 0x2000_0000;
const MAIN_COUNT_ADDR: u64 = 0x2000_0004;
const INITIAL_SP: u32 = 0x2000_8000;

// The GRTC's INTEN group 0 pends GRTC_0 = irq_base. Use NVIC IRQ 16 so the
// exception number (16 + 16 = 32) and its vector (32 * 4 = 0x80) stay in a
// compact hand-built vector table.
const GRTC_IRQ: u32 = 16;

// GRTC register absolute addresses (base + MDK offset).
const R_EVENTS_COMPARE0: u32 = GRTC_BASE + 0x100;
const R_INTENSET0: u32 = GRTC_BASE + 0x304;
const R_MODE: u32 = GRTC_BASE + 0x510;
const R_CC0_CCL: u32 = GRTC_BASE + 0x520;
const R_CC0_CCH: u32 = GRTC_BASE + 0x524;
const R_CC0_CCADD: u32 = GRTC_BASE + 0x528;
const R_SYSCOUNTERL: u32 = GRTC_BASE + 0x720;
const NVIC_ISER0: u32 = 0xE000_E100;

// SYSCOUNTER compare period, in SYSCOUNTER ticks (= 128 CPU cycles each on this
// profile). Small so several fires land inside the probe window.
const PERIOD: u32 = 3;

// ── A tiny two-pass Thumb assembler ─────────────────────────────────────────
//
// Hand-writing pc-relative `ldr` offsets by eye is where these fixtures go
// wrong; this resolves labels and literal-pool slots mechanically instead.

#[derive(Clone)]
enum Ins {
    Raw(u16),
    /// `ldr rd, [pc, #imm]` loading pool word `key`.
    LdrPool(u8, &'static str),
    /// unconditional `b label`.
    B(&'static str),
    Label(&'static str),
}

struct Asm {
    base: u32,
    ins: Vec<Ins>,
    pool: Vec<(&'static str, u32)>,
}

impl Asm {
    fn new(base: u32) -> Self {
        Asm {
            base,
            ins: Vec::new(),
            pool: Vec::new(),
        }
    }
    fn movs(&mut self, rd: u8, imm: u8) -> &mut Self {
        self.ins
            .push(Ins::Raw(0x2000 | ((rd as u16) << 8) | imm as u16));
        self
    }
    fn lsls(&mut self, rd: u8, rm: u8, imm5: u8) -> &mut Self {
        self.ins.push(Ins::Raw(
            (imm5 as u16) << 6 | ((rm as u16) << 3) | rd as u16,
        ));
        self
    }
    fn adds(&mut self, rd: u8, imm: u8) -> &mut Self {
        self.ins
            .push(Ins::Raw(0x3000 | ((rd as u16) << 8) | imm as u16));
        self
    }
    /// `str rt, [rn, #0]`.
    fn str0(&mut self, rt: u8, rn: u8) -> &mut Self {
        self.ins
            .push(Ins::Raw(0x6000 | ((rn as u16) << 3) | rt as u16));
        self
    }
    /// `ldr rt, [rn, #0]`.
    fn ldr0(&mut self, rt: u8, rn: u8) -> &mut Self {
        self.ins
            .push(Ins::Raw(0x6800 | ((rn as u16) << 3) | rt as u16));
        self
    }
    fn ldr_pool(&mut self, rd: u8, key: &'static str) -> &mut Self {
        self.ins.push(Ins::LdrPool(rd, key));
        self
    }
    fn wfi(&mut self) -> &mut Self {
        self.ins.push(Ins::Raw(0xBF30));
        self
    }
    fn bx(&mut self, rm: u8) -> &mut Self {
        self.ins.push(Ins::Raw(0x4700 | ((rm as u16) << 3)));
        self
    }
    fn b(&mut self, label: &'static str) -> &mut Self {
        self.ins.push(Ins::B(label));
        self
    }
    fn label(&mut self, name: &'static str) -> &mut Self {
        self.ins.push(Ins::Label(name));
        self
    }
    fn word(&mut self, key: &'static str, val: u32) -> &mut Self {
        self.pool.push((key, val));
        self
    }

    /// Assemble to (entry_addr, bytes). The literal pool is appended after the
    /// code, word-aligned.
    fn assemble(&self) -> (u32, Vec<u8>) {
        // Pass 1: assign an address to each instruction and every label.
        let mut addr = self.base;
        let mut labels: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
        for i in &self.ins {
            match i {
                Ins::Label(n) => {
                    labels.insert(n, addr);
                }
                _ => addr += 2,
            }
        }
        // Pool starts word-aligned after the code.
        let pool_start = (addr + 3) & !3;
        let mut pool_addr: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
        for (idx, (k, _)) in self.pool.iter().enumerate() {
            pool_addr.insert(k, pool_start + (idx as u32) * 4);
        }

        // Pass 2: emit.
        let mut out: Vec<u8> = Vec::new();
        let mut pc = self.base;
        let emit = |out: &mut Vec<u8>, hw: u16| out.extend_from_slice(&hw.to_le_bytes());
        for i in &self.ins {
            match i {
                Ins::Label(_) => {}
                Ins::Raw(hw) => {
                    emit(&mut out, *hw);
                    pc += 2;
                }
                Ins::LdrPool(rd, key) => {
                    let target = pool_addr[key];
                    let base = (pc + 4) & !3;
                    let imm = (target - base) / 4;
                    assert!(imm <= 255, "ldr literal out of range for {key}");
                    emit(&mut out, 0x4800 | ((*rd as u16) << 8) | imm as u16);
                    pc += 2;
                }
                Ins::B(label) => {
                    let target = labels[label];
                    let off = (target as i64 - (pc as i64 + 4)) / 2;
                    let imm11 = (off as i32 as u32) & 0x7FF;
                    emit(&mut out, 0xE000 | imm11 as u16);
                    pc += 2;
                }
            }
        }
        // Pad to the word-aligned pool start with nop halfwords (all emitted
        // code is 2-byte aligned, so this lands exactly on `pool_start`).
        while (self.base + out.len() as u32) < pool_start {
            emit(&mut out, 0xBF00); // nop
        }
        for (_, v) in &self.pool {
            out.extend_from_slice(&v.to_le_bytes());
        }
        (self.base, out)
    }
}

fn write_bytes(bus: &mut SystemBus, base: u32, bytes: &[u8]) {
    for (i, b) in bytes.iter().enumerate() {
        bus.write_u8(base as u64 + i as u64, *b).unwrap();
    }
}

fn write_word(bus: &mut SystemBus, addr: u64, word: u32) {
    bus.write_u32(addr, word).unwrap();
}

/// Build the shared machine assembly: a bare `SystemBus` + `configure_cortex_m`
/// (real SCB/NVIC/DWT), with a native GRTC registered at [`GRTC_BASE`]. The
/// GRTC gets the bus cycle clock through the standard `add_peripheral` attach
/// choke; `scheduler = false` pins it back onto the legacy walk to form the
/// reference lane from the identical assembly.
fn build_machine(scheduler: bool, tick_interval: u32) -> Machine<CortexM> {
    let mut bus = SystemBus::new();
    let (cpu, _nvic) = configure_cortex_m(&mut bus);

    // 12 CC channels, group g → NVIC IRQ (GRTC_IRQ + g).
    let grtc = Nrf54lGrtc::new_with_cc_and_irq(12, GRTC_IRQ);
    bus.add_peripheral(
        "grtc",
        GRTC_BASE as u64,
        0x1000,
        Some(GRTC_IRQ),
        Box::new(grtc),
    );

    if !scheduler {
        let idx = bus.find_peripheral_index_by_name("grtc").unwrap();
        bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Nrf54lGrtc>()
            .unwrap()
            .force_legacy_walk();
    }

    let mut machine = Machine::new(cpu, bus);
    machine.config.peripheral_tick_interval = tick_interval;
    machine.bus.config.peripheral_tick_interval = tick_interval;
    machine.cpu.sp = INITIAL_SP;
    machine
}

/// The shared ISR: clear EVENTS_COMPARE[0], re-arm the next compare via CCADD
/// (CC0 += PERIOD off the live SYSCOUNTER, which also re-arms the channel —
/// exactly nrfx's periodic-tick idiom), and increment the ISR counter.
fn assemble_isr(base: u32) -> (u32, Vec<u8>) {
    let mut a = Asm::new(base);
    // clear the event (write 0).
    a.ldr_pool(0, "events0").movs(1, 0).str0(1, 0);
    // CCADD = PERIOD (REFERENCE = SYSCOUNTER, bit 31 clear) → CC0 = now + PERIOD, armed.
    a.ldr_pool(0, "ccadd").movs(1, PERIOD as u8).str0(1, 0);
    // isr_count++.
    a.ldr_pool(0, "isrcnt").ldr0(1, 0).adds(1, 1).str0(1, 0);
    a.bx(14);
    a.word("events0", R_EVENTS_COMPARE0)
        .word("ccadd", R_CC0_CCADD)
        .word("isrcnt", ISR_COUNT_ADDR as u32);
    a.assemble()
}

/// Main firmware: enable the NVIC line, arm CC0 at PERIOD, enable INTEN0, start
/// the SYSCOUNTER, then spin incrementing the main counter and polling
/// SYSCOUNTERL (into r4 — the derived-read surface the differential pins).
fn assemble_main(base: u32, with_wfi: bool) -> (u32, Vec<u8>) {
    let mut a = Asm::new(base);
    // NVIC ISER0 |= 1 << GRTC_IRQ.
    a.ldr_pool(5, "iser")
        .movs(1, 1)
        .lsls(1, 1, GRTC_IRQ as u8)
        .str0(1, 5);
    // Arm CC0 at PERIOD: CCL = PERIOD (disarms), CCH = 0 (arms).
    a.ldr_pool(5, "ccl").movs(1, PERIOD as u8).str0(1, 5);
    a.ldr_pool(5, "cch").movs(1, 0).str0(1, 5);
    // INTENSET0 = COMPARE0.
    a.ldr_pool(5, "intenset").movs(1, 1).str0(1, 5);
    // MODE = SYSCOUNTEREN (start the counter).
    a.ldr_pool(5, "mode").movs(1, 2).str0(1, 5);
    // Main loop.
    a.ldr_pool(2, "maincnt").ldr_pool(6, "sysl").movs(3, 0);
    a.label("loop");
    if with_wfi {
        a.wfi();
    }
    a.adds(3, 1).str0(3, 2).ldr0(4, 6).b("loop");
    a.word("iser", NVIC_ISER0)
        .word("ccl", R_CC0_CCL)
        .word("cch", R_CC0_CCH)
        .word("intenset", R_INTENSET0)
        .word("mode", R_MODE)
        .word("maincnt", MAIN_COUNT_ADDR as u32)
        .word("sysl", R_SYSCOUNTERL);
    a.assemble()
}

const MAIN_ENTRY: u32 = 0x140;
const ISR_ENTRY: u32 = 0xC0;

fn load_firmware(bus: &mut SystemBus, with_wfi: bool) {
    // Vector table: exception (16 + GRTC_IRQ) at (16 + GRTC_IRQ) * 4.
    write_word(bus, ((16 + GRTC_IRQ) * 4) as u64, ISR_ENTRY | 1);
    let (isr_base, isr) = assemble_isr(ISR_ENTRY);
    write_bytes(bus, isr_base, &isr);
    let (main_base, main) = assemble_main(MAIN_ENTRY, with_wfi);
    write_bytes(bus, main_base, &main);
    assert!(
        isr_base + isr.len() as u32 <= MAIN_ENTRY,
        "ISR image ({} bytes) overruns the main entry",
        isr.len()
    );
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Probe {
    step: u64,
    total_cycles: u64,
    pc: u32,
    regs: [u32; 16],
    isr_count: u32,
    main_count: u32,
}

fn probe(machine: &Machine<CortexM>, step: u64) -> Probe {
    let mut regs = [0u32; 16];
    for (i, r) in regs.iter_mut().enumerate() {
        *r = machine.read_core_reg(i as u8);
    }
    Probe {
        step,
        total_cycles: machine.total_cycles,
        pc: machine.get_pc(),
        regs,
        isr_count: machine.bus.read_u32(ISR_COUNT_ADDR).unwrap(),
        main_count: machine.bus.read_u32(MAIN_COUNT_ADDR).unwrap(),
    }
}

fn run_probed(machine: &mut Machine<CortexM>, entry: u32, steps: u64) -> Vec<Probe> {
    machine.cpu.pc = entry;
    let mut probes = Vec::with_capacity(steps as usize);
    for s in 0..steps {
        machine.run(Some(1)).unwrap();
        probes.push(probe(machine, s + 1));
    }
    probes
}

fn assert_probes_identical(reference: &[Probe], candidate: &[Probe], what: &str) {
    assert_eq!(reference.len(), candidate.len());
    for (r, c) in reference.iter().zip(candidate.iter()) {
        assert_eq!(
            r, c,
            "{what}: first divergence at step {} (walk-reference vs scheduler)",
            r.step
        );
    }
}

/// Gate 1: the compare-IRQ + SYSCOUNTER-poll firmware, walk-on vs scheduler at
/// tick interval 1 — every instruction-boundary observable byte-identical
/// (IRQ delivery cycles, ISR execution count, total_cycles, registers —
/// including r4, the raw SYSCOUNTER poll).
#[test]
fn grtc_compare_irq_firmware_is_byte_identical_at_interval_1() {
    const STEPS: u64 = 3_000;

    let mut walk = build_machine(false, 1);
    load_firmware(&mut walk.bus, false);
    let walk_probes = run_probed(&mut walk, MAIN_ENTRY, STEPS);

    let mut sched = build_machine(true, 1);
    load_firmware(&mut sched.bus, false);
    let sched_probes = run_probed(&mut sched, MAIN_ENTRY, STEPS);

    // The firmware must actually exercise the surface: repeated compare IRQs
    // and a live main loop that observed the SYSCOUNTER advancing.
    let last = walk_probes.last().unwrap();
    assert!(
        last.isr_count >= 3,
        "reference must take repeated GRTC compare IRQs (got {})",
        last.isr_count
    );
    assert!(last.main_count > 100, "main loop must run");
    assert!(
        last.regs[4] > 0,
        "the main loop must observe the SYSCOUNTER advance"
    );

    assert_probes_identical(&walk_probes, &sched_probes, "grtc compare-irq firmware");
}

/// Gate 2: scheduler @ interval 64 vs the walk-on interval-1 golden reference.
/// IRQ delivery quantises to the batch grid (≤ one interval, documented), so
/// per-instruction state is NOT compared — but the ISR count over the fixed
/// window must be EXACT (absolute compare deadlines, no drift), and the window
/// edge is verified to be more than one interval + entry-lag away from any
/// delivery in the reference.
#[test]
fn grtc_isr_count_is_exact_at_interval_64() {
    const STEPS: u64 = 6_000;

    let mut walk = build_machine(false, 1);
    load_firmware(&mut walk.bus, false);
    let walk_probes = run_probed(&mut walk, MAIN_ENTRY, STEPS);

    let mut sched = build_machine(true, 64);
    load_firmware(&mut sched.bus, false);
    sched.cpu.pc = MAIN_ENTRY;
    sched.run(Some(STEPS as u32)).unwrap();

    let reference = walk_probes.last().unwrap();
    let sched_isr = sched.bus.read_u32(ISR_COUNT_ADDR).unwrap();

    let last_delivery_step = walk_probes
        .iter()
        .zip(walk_probes.iter().skip(1))
        .filter(|(a, b)| b.isr_count > a.isr_count)
        .map(|(_, b)| b.step)
        .next_back()
        .expect("reference delivers at least one ISR");
    assert!(
        STEPS - last_delivery_step > 128 + 16,
        "fixture must keep the window edge > interval + entry lag from the last \
         delivery (last at step {last_delivery_step})"
    );

    assert!(
        reference.isr_count >= 3,
        "reference must take repeated IRQs"
    );
    assert_eq!(
        sched_isr, reference.isr_count,
        "ISR execution count over the fixed window must be exact at interval 64"
    );
    assert_eq!(
        sched.total_cycles, reference.total_cycles,
        "total_cycles must match (GRTC tick-cost normalized to zero)"
    );
}

/// Gate 3: WFI idle fast-forward + GRTC. The sleeping firmware must
/// fast-forward to the SCHEDULED compare deadline (`next_event_deadline` clamps
/// the FF budget) and wake correctly: identical architectural end state vs the
/// FF-off run at the same step budget, with far fewer retired instructions and
/// a large `idle_fast_forward_cycles_skipped`.
#[test]
fn wfi_sleep_wakes_from_scheduled_grtc_compare() {
    const STEPS: u32 = 8_000;

    let build = |ff: bool| -> Machine<CortexM> {
        let mut machine = build_machine(true, 1);
        load_firmware(&mut machine.bus, true);
        machine.config.idle_fast_forward_enabled = ff;
        // FF is only legal when nothing depends on the legacy walk; the GRTC is
        // scheduler-driven now, so delete the walk exactly like the other WFI
        // FF fixtures.
        machine.bus.legacy_walk_disabled = true;
        machine.cpu.pc = MAIN_ENTRY;
        machine
    };

    let mut ff_off = build(false);
    ff_off.run(Some(STEPS)).unwrap();
    let off_probe = probe(&ff_off, STEPS as u64);

    let mut ff_on = build(true);
    ff_on.reset_step_profile();
    ff_on.run(Some(STEPS)).unwrap();
    let on_probe = probe(&ff_on, STEPS as u64);

    // The pattern must actually sleep-and-wake repeatedly on the scheduled GRTC
    // compares.
    assert!(
        off_probe.isr_count >= 5,
        "WFI loop must take repeated GRTC compare IRQs (got {})",
        off_probe.isr_count
    );

    // Fast-forward must not change WHEN things happen or HOW OFTEN the ISR
    // runs — only how many instructions the CPU retires getting there.
    assert_eq!(on_probe.total_cycles, off_probe.total_cycles);
    assert_eq!(
        on_probe.isr_count, off_probe.isr_count,
        "every scheduled GRTC compare must wake the sleeping core and run the ISR"
    );

    let skipped = ff_on.idle_fast_forward_cycles_skipped;
    assert!(
        skipped > (STEPS as u64) / 2,
        "idle fast-forward must skip most of the idle window (skipped {skipped} of {STEPS})"
    );

    let retired = ff_on.step_profile().cpu_instructions;
    assert!(
        retired < (STEPS as u64) / 4,
        "idle fast-forward must skip the sleeping cycles ({retired} retired of {STEPS})"
    );
}
