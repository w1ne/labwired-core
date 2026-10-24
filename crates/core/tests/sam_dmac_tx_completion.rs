// SPDX-License-Identifier: MIT
//! Authored TX driver ordering, not CODAL events/fibers or native Arcade.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{
    bus::SystemBus, cycle_clock::event_scheduler_enabled, memory::ProgramImage,
    peripherals::spi::SpiDevice, Arch, Bus, Machine, SimulationObserver,
};
use std::sync::{Arc, Mutex};

const DMA: u32 = 0x4100a000;
const SPI: u32 = 0x43000000;
const PORT: u32 = 0x41008080;
const AHB: u32 = 0x40000810;
const APBD: u32 = 0x40000820;
const GCLK: u32 = 0x40001c00 + 0x80 + 34 * 4;
const TABLE: u32 = 0x20001000;
const WB: u32 = 0x20001200;
const SOURCE: u32 = 0x20002000;
const MARK: u32 = 0x20000000;
const CALLBACK: u32 = 0x7000;
const TX: u32 = (0x0d << 8) | (2 << 20);
const MASTER: u32 = (3 << 2) | (2 << 16);
const MODES: &[&str] = &[
    "complete",
    "premature",
    "primask",
    "dma-nvic",
    "dma-inten",
    "txc-nvic",
    "txc-inten",
    "pause-spi",
    "pause-gclk",
    "reset",
    "read-error",
];

#[derive(Clone, Debug)]
struct Event {
    step: usize,
    kind: &'static str,
    value: u32,
}

#[derive(Debug, Default)]
struct Trace {
    step: Mutex<usize>,
    events: Mutex<Vec<Event>>,
    return_pc: Mutex<u32>,
}
impl Trace {
    fn record(&self, kind: &'static str, value: u32) {
        let step = *self.step.lock().unwrap();
        self.events
            .lock()
            .unwrap()
            .push(Event { step, kind, value });
    }
}
impl SimulationObserver for Trace {
    fn on_memory_write(&self, address: u64, _old: u8, value: u8) {
        if address == u64::from(SPI + 0x28) {
            self.record("accept", u32::from(value));
        } else if address == u64::from(PORT + 0x18) && value & 0x80 != 0 {
            self.record("cs-store", u32::from(value));
        }
    }
    fn on_step_start(&self, pc: u32, _opcode: u32) {
        if pc == CALLBACK {
            self.record("callback-entry", pc);
        }
        if pc == *self.return_pc.lock().unwrap() {
            self.record("callback-return", pc);
        }
    }
}

struct Slave(Arc<Trace>);
impl SpiDevice for Slave {
    fn cs_pin(&self) -> &str {
        "PB7"
    }
    fn cs_select(&mut self) {
        self.0.record("select", 0);
    }
    fn cs_release(&mut self) {
        self.0.record("release", 0);
    }
    fn transfer(&mut self, byte: u8) -> u8 {
        self.0.record("wire", u32::from(byte));
        0xff
    }
}

fn board(legacy: bool, trace: Arc<Trace>) -> SystemBus {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut chip =
        ChipDescriptor::from_file(root.join("configs/chips/atsamd51-pybadge.yaml")).unwrap();
    chip.peripherals.push(serde_yaml::from_str(
        "id: tx-dma\ntype: sam_dmac_sercom4_tx_byte\nbase_address: 0x4100a000\nsize: 1KB\nirq: 31\nclock: { controller: mclk, reg: AHBMASK, bit: 9 }\n"
    ).unwrap());
    let manifest = SystemManifest::from_file(root.join("configs/systems/pybadge.yaml")).unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    if legacy {
        assert!(bus
            .peripherals
            .iter_mut()
            .find(|p| p.name == "sercom4")
            .unwrap()
            .dev
            .force_legacy_walk());
        bus.recompute_walk_deletable();
        bus.refresh_peripheral_index();
    }
    bus.attach_spi_device("sercom4", Box::new(Slave(trace)))
        .unwrap();
    bus
}

#[derive(Default)]
struct Guest(Vec<u8>);
impl Guest {
    fn op(&mut self, op: u16) {
        self.0.extend(op.to_le_bytes());
    }
    fn align(&mut self) {
        if self.0.len() & 3 != 0 {
            self.op(0xbf00);
        }
    }
    fn load0(&mut self, value: u32) {
        self.align();
        self.op(0x4800);
        self.op(0xe001);
        self.0.extend(value.to_le_bytes());
    }
    fn store(&mut self, address: u32, value: u32, byte: bool) {
        self.align();
        for op in [0x4801, 0x4902, if byte { 0x7001 } else { 0x6001 }, 0xe003] {
            self.op(op);
        }
        self.0.extend(address.to_le_bytes());
        self.0.extend(value.to_le_bytes());
    }
    fn sample(&mut self, address: u32, witness: u32, byte: bool) {
        self.load0(address);
        self.op(if byte { 0x7801 } else { 0x8801 });
        self.load0(witness);
        self.op(0x6001);
    }
    fn wait_zero_count(&mut self) {
        self.load0(WB + 2);
        for op in [0x8801, 0x2900, 0xd1fc] {
            self.op(op);
        }
    }
    fn wait_error_irq(&mut self) {
        self.load0(MARK + 0x0c);
        for op in [0x6801, 0x2901, 0xd1fc] {
            self.op(op);
        }
    }
    fn wait_txc(&mut self) {
        self.load0(SPI + 0x18);
        for op in [0x7801, 0x2202, 0x4211, 0xd0fb] {
            self.op(op);
        }
    }
}

fn guest(mode: &str) -> (ProgramImage, u32) {
    let mut g = Guest::default();
    g.op(0xb672); // CPSID i, all setup and DMA descriptor writes by the CPU.
    for (address, value) in [
        (0xe000ed08, 0x4000),
        (AHB, 1 << 9),
        (APBD, 1),
        (GCLK, 1 << 6),
        (SOURCE, 0x64534231),
        (0x2002ffff, 0x5a),
    ] {
        g.store(address, value, address == 0x2002ffff);
    }
    for offset in (0..0x30).step_by(4) {
        g.store(MARK + offset, 0, false);
    }
    for (address, value, byte) in [
        (PORT + 0x36, 0x20, true),
        (PORT + 0x37, 0x20, true),
        (PORT + 0x4d, 1, true),
        (PORT + 0x4f, 1, true),
        (PORT, (1 << 7) | (1 << 5), false),
        (SPI, MASTER, false),
        (SPI + 4, 0, false),
        // 4096 nominal cycles/byte leaves a real completion-observation
        // window even at the largest admitted 1024-cycle service interval.
        // This slow diagnostic is not a representative throughput workload.
        (SPI + 0x0c, 255, true),
        (SPI, MASTER | 2, false),
    ] {
        g.store(address, value, byte);
    }
    if mode != "dma-nvic" {
        g.store(0xe000e100, 1 << 31, false);
    }
    if mode != "txc-nvic" {
        g.store(0xe000e104, 1 << 31, false);
    }
    if mode != "txc-inten" {
        g.store(SPI + 0x16, 2, true);
    }
    for (address, value) in [
        (TABLE, (4 << 16) | 0x0409),
        (
            TABLE + 4,
            if mode == "read-error" {
                0x20030003
            } else {
                SOURCE + 4
            },
        ),
        (TABLE + 8, SPI + 0x28),
        (TABLE + 12, 0),
        (WB, 0xaaaaaaaa),
        (WB + 4, 0xaaaaaaaa),
        (WB + 8, 0xaaaaaaaa),
        (WB + 12, 0xaaaaaaaa),
        (DMA + 0x34, TABLE),
        (DMA + 0x38, WB),
        (DMA, 0x102),
    ] {
        g.store(address, value, false);
    }
    g.store(DMA + 0x4d, if mode == "dma-inten" { 0 } else { 3 }, true);
    g.store(DMA + 0x40, TX | 2, false);
    if mode != "primask" {
        g.op(0xb662);
    }
    // NO software trigger, host request or host IRQ injection.
    if mode == "read-error" {
        g.wait_error_irq();
    } else {
        g.wait_zero_count();
        g.sample(SPI + 0x18, MARK + 0x18, true);
    }
    if let Some((gate, enable)) = match mode {
        "pause-spi" => Some((APBD, 1)),
        "pause-gclk" => Some((GCLK, 1 << 6)),
        _ => None,
    } {
        g.store(gate, 0, false);
        g.store(MARK + 0x10, 1, false);
        for _ in 0..256 {
            g.op(0xbf00);
        }
        g.store(MARK + 0x14, 1, false);
        g.store(gate, enable, false);
    }
    let abort = matches!(mode, "reset" | "read-error");
    let return_pc;
    if abort {
        g.store(SPI, 1, false); // Discard queued, unshifted bytes; no success call.
        g.store(MARK, 0xab07, false);
        return_pc = u32::MAX;
    } else {
        if mode != "premature" {
            g.wait_txc();
        }
        g.store(PORT + 0x18, 1 << 7, false);
        g.load0(CALLBACK | 1);
        g.op(0x4780); // Actual BLX r0; the callee is a separate guest segment.
        return_pc = 0x4200 + g.0.len() as u32;
        g.store(MARK, 0x600d, false);
    }
    g.op(0xe7fe);
    let dma_handler = 0x4200 + g.0.len() as u32;
    g.sample(DMA + 0x4e, MARK + 0x0c, true);
    g.store(MARK + 4, 1, false);
    g.store(DMA + 0x4e, 3, true);
    g.op(0x4770); // BX LR / exception return.
    let spi_handler = 0x4200 + g.0.len() as u32;
    g.sample(SPI + 0x18, MARK + 0x20, true);
    g.store(MARK + 8, 1, false);
    g.store(SPI + 0x14, 2, true); // INTENCLR; leave TXC for driver polling.
    g.op(0x4770);
    let mut bytes = vec![0; 0x200];
    bytes[..4].copy_from_slice(&0x20004000u32.to_le_bytes());
    bytes[4..8].copy_from_slice(&0x4201u32.to_le_bytes());
    for (irq, address) in [(31, dma_handler), (63, spi_handler)] {
        let offset = (16 + irq) * 4;
        bytes[offset..offset + 4].copy_from_slice(&(address | 1).to_le_bytes());
    }
    bytes.extend(g.0);
    let mut callback = Guest::default();
    callback.store(MARK + 0x24, 1, false);
    callback.op(0x4770); // Actual callback return to the BLX caller.
    let mut image = ProgramImage::new(0x4201, Arch::Arm);
    image.add_segment(0x4000, bytes);
    image.add_segment(u64::from(CALLBACK), callback.0);
    (image, return_pc)
}

fn ordering(events: &[Event]) -> Result<(), &'static str> {
    let positions = |kind| {
        events
            .iter()
            .enumerate()
            .filter(|(_, e)| e.kind == kind)
            .map(|(i, _)| i)
            .collect::<Vec<_>>()
    };
    let accepted = positions("accept");
    let wire = positions("wire");
    let store = positions("cs-store");
    if wire.len() != 4 || store.len() != 1 || store[0] <= wire[3] {
        return Err("cs-before-final-wire");
    }
    let dma = positions("dma-complete");
    let txc = positions("txc");
    let release = positions("release");
    let enter = positions("callback-entry");
    let returned = positions("callback-return");
    if accepted.len() != 4
        || dma.len() != 1
        || txc.len() != 1
        || release.len() != 1
        || enter.len() != 1
        || returned.len() != 1
    {
        return Err("missing-or-duplicate-boundary");
    }
    if !(accepted[3] < dma[0]
        && dma[0] < wire[3]
        && wire[3] < txc[0]
        && txc[0] < store[0]
        && store[0] < release[0]
        && release[0] < enter[0]
        && enter[0] < returned[0])
    {
        return Err("completion-order");
    }
    Ok(())
}

#[test]
fn cpu_tx_driver_separates_dma_wire_txc_cs_and_real_callback_with_premature_mutant() {
    for legacy in [true, false] {
        for interval in [1, 64, 1024] {
            if interval > 1 && (legacy || !event_scheduler_enabled()) {
                continue;
            }
            for &mode in MODES {
                let trace = Arc::new(Trace::default());
                let mut bus = board(legacy, trace.clone());
                let (cpu, _) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
                bus.config.peripheral_tick_interval = interval;
                let mut machine = Machine::new(cpu, bus);
                machine.config.peripheral_tick_interval = interval;
                let (image, resume) = guest(mode);
                *trace.return_pc.lock().unwrap() = resume;
                machine.add_observer(trace.clone());
                machine.load_firmware(&image).unwrap();
                let mut previous = 0;
                let mut entries = [0; 2];
                let mut returns = [0; 2];
                let mut complete_seen = false;
                let mut txc_seen = false;
                let mut gap = false;
                let mut pause_started = false;
                let mut pause_ended = false;
                for step in 0..20000 {
                    *trace.step.lock().unwrap() = step;
                    machine.step().unwrap();
                    let active = machine.cpu.active_exception;
                    if active != 0 && active != previous {
                        let index = match active {
                            47 => 0,
                            79 => 1,
                            _ => panic!("unexpected exception{active}"),
                        };
                        entries[index] += 1;
                        trace.record(if index == 0 { "dma-entry" } else { "spi-entry" }, active);
                    }
                    if previous != 0 && previous != active {
                        returns[usize::from(previous == 79)] += 1;
                        trace.record(
                            if previous == 79 {
                                "spi-return"
                            } else {
                                "dma-return"
                            },
                            previous,
                        );
                    }
                    previous = active;
                    let remaining = machine.bus.read_u16(u64::from(WB + 2)).unwrap();
                    let flags = machine.bus.read_u8(u64::from(DMA + 0x4e)).unwrap();
                    let spi_flags = machine.bus.read_u8(u64::from(SPI + 0x18)).unwrap();
                    if !complete_seen && remaining == 0 && flags & 2 != 0 {
                        // Both are labelled end-of-step observations; equal
                        // step numbers do not claim a physical sub-step order.
                        trace.record("writeback-zero", u32::from(remaining));
                        trace.record("dma-complete", u32::from(flags));
                        complete_seen = true;
                        gap = spi_flags & 2 == 0
                            && machine.bus.read_gpio_output_by_label("PB7") == Some(false);
                    }
                    if !txc_seen && spi_flags & 2 != 0 {
                        trace.record("txc", u32::from(spi_flags));
                        txc_seen = true;
                    }
                    // RAM word stores do not publish bus on_memory_write on
                    // this path. Observe the CPU-written witnesses read-only,
                    // just as count/flags above; do not inject or infer them
                    // from an anticipated guest PC or elapsed host time.
                    let start = machine.bus.read_u32(u64::from(MARK + 0x10)).unwrap();
                    let end = machine.bus.read_u32(u64::from(MARK + 0x14)).unwrap();
                    if !pause_started && start != 0 {
                        trace.record("pause-start", start);
                        pause_started = true;
                    }
                    if !pause_ended && end != 0 {
                        trace.record("pause-end", end);
                        pause_ended = true;
                    }
                }
                let events = trace.events.lock().unwrap().clone();
                let values = |kind| {
                    events
                        .iter()
                        .filter(|e| e.kind == kind)
                        .map(|e| e.value)
                        .collect::<Vec<_>>()
                };
                let accepted = values("accept");
                let wire = values("wire");
                let abort = matches!(mode, "reset" | "read-error");
                let dma_irq = !matches!(mode, "primask" | "dma-nvic" | "dma-inten");
                let spi_irq = !abort && !matches!(mode, "primask" | "txc-nvic" | "txc-inten");
                assert_eq!(
                    entries,
                    [usize::from(dma_irq), usize::from(spi_irq)],
                    "mode={mode}"
                );
                assert_eq!(returns, entries);
                assert_eq!(
                    machine.bus.read_u32(u64::from(MARK + 4)).unwrap(),
                    u32::from(dma_irq)
                );
                assert_eq!(
                    machine.bus.read_u32(u64::from(MARK + 8)).unwrap(),
                    u32::from(spi_irq)
                );
                assert_eq!(
                    machine.bus.read_u32(u64::from(MARK + 0x0c)).unwrap(),
                    if dma_irq {
                        if mode == "read-error" {
                            1
                        } else {
                            2
                        }
                    } else {
                        0
                    }
                );
                if spi_irq {
                    assert_eq!(machine.bus.read_u32(u64::from(MARK + 0x20)).unwrap() & 2, 2);
                }
                assert_eq!(machine.cpu.active_exception, 0);
                assert_eq!(
                    machine.bus.read_u32(u64::from(MARK)).unwrap(),
                    if abort { 0xab07 } else { 0x600d }
                );
                assert_eq!(machine.bus.read_u32(0xe000ed28).unwrap(), 0);
                assert_eq!(machine.bus.read_u32(0xe000ed2c).unwrap(), 0);
                let verdict;
                if abort {
                    assert!(values("callback-entry").is_empty());
                    assert!(values("callback-return").is_empty());
                    assert!(values("cs-store").is_empty());
                    assert!(!txc_seen);
                    assert_eq!(
                        accepted,
                        if mode == "reset" {
                            vec![0x31, 0x42, 0x53, 0x64]
                        } else {
                            vec![0x5a]
                        }
                    );
                    assert_eq!(
                        wire,
                        if mode == "reset" {
                            vec![0x31, 0x42]
                        } else {
                            vec![]
                        }
                    );
                    assert_eq!(machine.bus.read_u32(u64::from(MARK + 0x24)).unwrap(), 0);
                    verdict = "abort-no-success";
                } else {
                    assert_eq!(accepted, [0x31, 0x42, 0x53, 0x64]);
                    assert!(
                        gap,
                        "no actual DMA-complete/TXC-low/CS-selected window: mode={mode}"
                    );
                    assert_eq!(machine.bus.read_u32(u64::from(MARK + 0x18)).unwrap() & 2, 0);
                    assert_eq!(machine.bus.read_u32(u64::from(MARK + 0x24)).unwrap(), 1);
                    if mode == "premature" {
                        assert_eq!(ordering(&events), Err("cs-before-final-wire"));
                        assert_eq!(wire, [0x31, 0x42]);
                        verdict = "refused-cs-before-final-wire";
                    } else {
                        assert_eq!(wire, [0x31, 0x42, 0x53, 0x64]);
                        assert_eq!(ordering(&events), Ok(()), "mode={mode} events={events:?}");
                        verdict = "ordered";
                    }
                }
                if mode.starts_with("pause-") {
                    let start = events.iter().find(|e| e.kind == "pause-start").unwrap();
                    let end = events.iter().find(|e| e.kind == "pause-end").unwrap();
                    assert!(end.step > start.step + 128);
                    assert!(!events.iter().any(|e| e.step > start.step
                        && e.step < end.step
                        && matches!(e.kind, "wire" | "txc" | "release" | "callback-entry")));
                }
                println!("SAM_TX_ORDER legacy={legacy} interval={interval} mode={mode} verdict={verdict} accepted={} wire={} dma_entries={} dma_returns={} spi_entries={} spi_returns={} callbacks={} callback_returns={} gap={gap}",
                    accepted.len(), wire.len(), entries[0], returns[0], entries[1], returns[1], values("callback-entry").len(), values("callback-return").len());
                for (order, event) in events.iter().enumerate() {
                    println!("SAM_TX_BOUNDARY legacy={legacy} interval={interval} mode={mode} order={order} step={} kind={} value={}", event.step, event.kind, event.value);
                }
            }
        }
    }
}
