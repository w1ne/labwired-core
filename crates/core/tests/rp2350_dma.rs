// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::rp2040::dma::{DmaLayout, Rp2040Dma};
use labwired_core::{Bus, Peripheral};

const SRC: u32 = 0x2000_1000;
const DST: u32 = 0x2000_2000;
fn dma() -> Rp2040Dma {
    Rp2040Dma::with_layout(DmaLayout::Rp2350)
}
fn ctrl(channel: u32) -> u32 {
    1 | (2 << 2) | (1 << 4) | (1 << 6) | (channel << 13) | (63 << 17)
}
fn setup(d: &mut Rp2040Dma, channel: u64, count: u32, control: u32) {
    let base = channel * 0x40;
    for (offset, value) in [(0, SRC), (4, DST), (8, count), (12, control)] {
        d.write_u32(base + offset, value).unwrap();
    }
}
fn tick(d: &mut Rp2040Dma, b: &mut SystemBus, n: usize) {
    for _ in 0..n {
        d.tick_with_bus_forced(b);
    }
}

#[test]
fn chip_profiles_have_sixteen_channels_and_rp2040_keeps_twelve() {
    for name in ["rp2350", "rp2350-feather", "rp2354b", "rp2040"] {
        let chip_path = root(&format!("configs/chips/{name}.yaml"));
        let chip = ChipDescriptor::from_file(&chip_path).unwrap();
        let mut system =
            SystemManifest::from_file(root("configs/systems/rp2350-zero.yaml")).unwrap();
        system.chip = chip_path.to_str().unwrap().into();
        let b = SystemBus::from_config(&chip, &system).unwrap();
        let offset = if name == "rp2040" { 0x448 } else { 0x468 };
        assert_eq!(
            b.read_u32(0x5000_0000 + offset).unwrap(),
            if name == "rp2040" { 12 } else { 16 },
            "{name}"
        );
    }
}

#[test]
fn native_channel_fifteen_and_busy_bit_move_real_words() {
    let mut d = dma();
    let mut b = SystemBus::new();
    b.write_u32(SRC.into(), 0x4433_2211).unwrap();
    setup(&mut d, 15, 1, ctrl(15));
    assert_ne!(d.read_u32(0x3cc).unwrap() & (1 << 26), 0);
    assert_eq!(
        d.read_u32(0x3cc).unwrap() & (1 << 24),
        0,
        "BSWAP is not BUSY"
    );
    tick(&mut d, &mut b, 1);
    assert_eq!(b.read_u32(DST.into()).unwrap(), 0x4433_2211);
    assert_eq!(d.read_u32(0x3c8).unwrap(), 0);
    assert_eq!(d.read_u32(0x400).unwrap(), 1 << 15);
}

#[test]
fn four_native_irq_lines_have_independent_mask_force_and_ack() {
    let mut d = dma();
    for line in 0..4 {
        let reg = 0x404 + line * 0x10;
        d.write_u32(reg + 4, 1 << 15).unwrap();
        assert_eq!(
            d.read_u32(reg + 8).unwrap(),
            1 << 15,
            "forced IRQ ignores INTE"
        );
        assert_eq!(
            d.tick_elapsed_forced(1).explicit_irqs,
            Some(vec![10 + line as u32])
        );
        d.write_u32(reg + 8, 1 << 15).unwrap();
        assert_ne!(d.read_u32(reg + 8).unwrap(), 0, "W1C does not clear INTF");
        d.write_u32(reg + 4, 0).unwrap();
    }
    d.write_u32(0x434, u32::MAX).unwrap();
    assert_eq!(d.read_u32(0x434).unwrap(), 0xffff);
    let mut b = SystemBus::new();
    b.write_u32(SRC.into(), 7).unwrap();
    setup(&mut d, 15, 1, ctrl(15));
    tick(&mut d, &mut b, 1);
    assert_eq!(d.tick_elapsed_forced(1).explicit_irqs, Some(vec![13]));
    d.write_u32(0x43c, 1 << 15).unwrap();
    assert!(d.tick_elapsed_forced(1).explicit_irqs.is_none());
}

#[test]
fn native_reverse_and_skip_increments_preserve_untouched_slots() {
    let mut d = dma();
    let mut b = SystemBus::new();
    for i in 0..3 {
        b.write_u32(u64::from(SRC + 4 * i), 0x10 + i).unwrap();
    }
    setup(&mut d, 0, 3, (ctrl(0) | (1 << 5) | (1 << 7)) & !(1 << 6));
    d.write_u32(0, SRC + 8).unwrap();
    tick(&mut d, &mut b, 3);
    for i in 0..3 {
        assert_eq!(b.read_u32(u64::from(DST + 8 * i)).unwrap(), 0x12 - i);
        assert_eq!(b.read_u32(u64::from(DST + 8 * i + 4)).unwrap(), 0);
    }
    assert_eq!(d.read_u32(0).unwrap(), SRC - 4);
    assert_eq!(d.read_u32(4).unwrap(), DST + 24);
}

#[test]
fn native_ring_and_bswap_use_shifted_fields() {
    let mut d = dma();
    let mut b = SystemBus::new();
    for i in 0..3 {
        b.write_u32(u64::from(SRC + 4 * i), 0x1122_3300 + i)
            .unwrap();
    }
    setup(&mut d, 0, 3, ctrl(0) | (3 << 8) | (1 << 12) | (1 << 24));
    tick(&mut d, &mut b, 3);
    assert_eq!(b.read_u32(DST.into()).unwrap(), 0x0233_2211);
    assert_eq!(b.read_u32(u64::from(DST + 4)).unwrap(), 0x0133_2211);
    assert_eq!(d.read_u32(4).unwrap(), DST + 4);
}

#[test]
fn transfer_count_modes_reload_or_run_endlessly_until_native_abort() {
    let mut d = dma();
    let mut b = SystemBus::new();
    for i in 0..6 {
        b.write_u32(u64::from(SRC + i * 4), 20 + i).unwrap();
    }
    setup(&mut d, 0, 0x1000_0002, ctrl(0));
    tick(&mut d, &mut b, 3);
    assert_eq!(d.read_u32(8).unwrap(), 0x1000_0001);
    assert_eq!(d.read_u32(0x804).unwrap(), 0x1000_0002);
    assert_eq!(b.read_u32(u64::from(DST + 8)).unwrap(), 22);
    assert_eq!(d.read_u32(0x400).unwrap(), 1);
    assert!(
        d.write_u32(0x444, 1).is_err(),
        "old abort offset is native timer 1"
    );
    assert_ne!(d.read_u32(12).unwrap() & (1 << 26), 0);
    d.write_u32(0x464, 1).unwrap();
    setup(&mut d, 0, 0xf000_0001, ctrl(0));
    d.write_u32(0x400, 1).unwrap();
    tick(&mut d, &mut b, 3);
    assert_eq!(d.read_u32(8).unwrap(), 0xf000_0001);
    assert_eq!(d.read_u32(0x400).unwrap(), 0);
    d.write_u32(0x464, 1).unwrap();
    assert_eq!(d.read_u32(12).unwrap() & (1 << 26), 0);
}

#[test]
fn bus_errors_latch_native_error_irq_without_reporting_false_progress() {
    for bad_read in [true, false] {
        let mut d = dma();
        let mut b = SystemBus::new();
        b.write_u32(SRC.into(), 0x1234).unwrap();
        setup(&mut d, 0, 1, ctrl(0) | (1 << 23));
        d.write_u32(if bad_read { 0 } else { 4 }, 0xdead_0000)
            .unwrap();
        tick(&mut d, &mut b, 1);
        let bit = if bad_read { 1 << 30 } else { 1 << 29 };
        assert_eq!(d.read_u32(12).unwrap() & 0xe000_0000, bit | (1 << 31));
        assert_eq!(d.read_u32(8).unwrap(), 1);
        assert_eq!(d.read_u32(0x400).unwrap(), 1, "error IRQ overrides quiet");
        assert_eq!(b.read_u32(DST.into()).unwrap(), 0);
        d.write_u32(12, bit).unwrap();
        assert_eq!(d.read_u32(12).unwrap() & 0xe000_0000, 0);
    }
}

#[test]
fn malformed_or_unmodeled_native_modes_fail_before_changing_state() {
    let mut d = dma();
    for (offset, value) in [
        (8, 0x2000_0001),
        (12, ctrl(0) | (3 << 2)),
        (12, ctrl(0) | (1 << 25)),
        (0x440, 1),
        (0x454, 1),
        (0x480, 0),
        (0x500, 1),
    ] {
        let before = d.read_u32(offset).unwrap();
        assert!(d.write_u32(offset, value).is_err(), "offset {offset:#x}");
        assert_eq!(d.read_u32(offset).unwrap(), before);
    }
    assert!(d.read_u32(0x470).is_err());
    assert!(d.write_u32(3, 1).is_err());
}

#[test]
fn pio_dma_machine_path_waits_for_space_and_copies_each_received_word_once() {
    use labwired_core::system::cortex_m::configure_cortex_m;
    use labwired_core::{Cpu, Machine};
    for (block, relocated) in [(0u64, false), (1, false), (2, false), (2, true)] {
        let path = root("configs/chips/rp2354b.yaml");
        let mut chip = ChipDescriptor::from_file(&path).unwrap();
        let bases = if relocated {
            [0x5100_0000u64, 0x5101_0000, 0x5102_0000]
        } else {
            [0x5020_0000, 0x5030_0000, 0x5040_0000]
        };
        if relocated {
            for (i, base) in bases.iter().enumerate() {
                chip.peripherals
                    .iter_mut()
                    .find(|p| p.id == format!("pio{i}"))
                    .unwrap()
                    .base_address = *base;
            }
            chip.peripherals
                .iter_mut()
                .find(|p| p.id == "dma")
                .unwrap()
                .config
                .insert(
                    "pio_dreq_bases".into(),
                    serde_yaml::to_value(bases).unwrap(),
                );
        }
        let system: SystemManifest =
            serde_yaml::from_str(&format!("name: dma-pio-test\nchip: {}\n", path.display()))
                .unwrap();
        let mut b = SystemBus::from_config(&chip, &system).unwrap();
        let (mut cpu, _) = configure_cortex_m(&mut b);
        b.write_u16(0x2000_3000, 0xe7fe).unwrap(); // real Thumb B . for peripheral clocks
        cpu.set_pc(0x2000_3000);
        cpu.set_sp(0x2000_4000);
        let mut m = Machine::new(cpu, b);
        let pio = bases[block as usize];
        for (i, instruction) in [0x80a0, 0xa0c7, 0x8020].into_iter().enumerate() {
            m.bus
                .write_u32(pio + 0x48 + 4 * i as u64, instruction)
                .unwrap();
        }
        m.bus.write_u32(pio + 0xcc, 2 << 12).unwrap(); // wrap 0..2
        for i in 0..12u32 {
            m.bus
                .write_u32(u64::from(SRC + i * 4), 0x1020_3040 + i)
                .unwrap();
        }
        let tx = 0x5000_0000 + 15 * 0x40;
        let rx = 0x5000_0000 + 14 * 0x40;
        let control = |channel: u32, request: u32| 1 | (2 << 2) | (channel << 13) | (request << 17);
        for (base, from, to, c) in [
            (
                rx,
                pio as u32 + 0x20,
                DST,
                control(14, block as u32 * 8 + 4) | (1 << 6),
            ),
            (
                tx,
                SRC,
                pio as u32 + 0x10,
                control(15, block as u32 * 8) | (1 << 4),
            ),
        ] {
            for (off, val) in [(0, from), (4, to), (8, 12), (12, c)] {
                m.bus.write_u32(base + off, val).unwrap();
            }
        }
        for _ in 0..100 {
            m.step().unwrap();
        }
        assert_eq!(
            m.bus.read_u32(tx + 8).unwrap(),
            8,
            "TX stopped at its actual four-word FIFO"
        );
        assert_eq!(
            m.bus.read_u32(rx + 8).unwrap(),
            12,
            "empty RX cannot fabricate DMA data"
        );
        assert_eq!(m.bus.read_u32(pio + 0xc).unwrap() & 0xff, 4);
        m.bus.write_u32(pio, 1).unwrap();
        for _ in 0..500 {
            m.step().unwrap();
        }
        assert_eq!(m.bus.read_u32(tx + 8).unwrap(), 0, "PIO{block} TX");
        assert_eq!(m.bus.read_u32(rx + 8).unwrap(), 0, "PIO{block} RX");
        for i in 0..12u32 {
            assert_eq!(
                m.bus.read_u32(u64::from(DST + i * 4)).unwrap(),
                0x1020_3040 + i
            );
        }
        assert_eq!(
            m.bus.read_u32(pio + 8).unwrap(),
            0,
            "no overrun or underflow"
        );
    }
}

#[test]
fn pio_word_and_narrow_fifo_accesses_are_single_native_operations() {
    use labwired_core::peripherals::pio::Pio;
    let mut p = Pio::new();
    p.write_u32(0x10, 0x1122_3344).unwrap();
    p.write_u16(0x10, 0xabcd).unwrap();
    p.write(0x10, 0x5a).unwrap();
    assert_eq!(p.tx_fifo[0], [0x1122_3344, 0xabcd_abcd, 0x5a5a_5a5a]);
    p.rx_fifo[0]
        .borrow_mut()
        .extend([0x1122_3344, 0x5566_7788, 0x99aa_bbcc]);
    assert_eq!(p.peek(0x20), Some(0x44));
    assert_eq!(p.read_u32(0x20).unwrap(), 0x1122_3344);
    assert_eq!(p.read_u16(0x20).unwrap(), 0x7788);
    assert_eq!(p.read(0x20).unwrap(), 0xcc);
    assert!(!p.dma_request_active(4));
    assert_eq!(p.read_u32(0x20).unwrap(), 0);
    assert_eq!(p.read_u32(8).unwrap() & (1 << 8), 1 << 8);
    p.write_u32(8, 1 << 8).unwrap();
    assert_eq!(p.read_u32(8).unwrap() & (1 << 8), 0);
    p.write_u32(0xd0, (1 << 31) | 0x000c_0000).unwrap(); // join into RX
    assert!(p.tx_fifo[0].is_empty());
    assert!(!p.dma_request_active(0), "joined-away TX never raises DREQ");
}

#[test]
fn pio_input_uses_the_external_pad_and_wakes_a_waiting_machine() {
    use labwired_core::peripherals::pio::Pio;
    use labwired_core::CycleClock;
    let mut p = Pio::new();
    let lines = p.pin_lines(true);
    p.attach_cycle_clock(CycleClock::default());
    p.write_u32(0x48, 0x20a0).unwrap(); // WAIT 1 PIN 0
    p.write_u32(0x4c, 0x4001).unwrap(); // IN PINS,1
    p.write_u32(0x50, 0x8020).unwrap(); // PUSH BLOCK
    p.write_u32(0xcc, 2 << 12).unwrap();
    p.write_u32(0xdc, 31 << 15).unwrap(); // IN_BASE=GPIO31
    p.write_u32(0, 1).unwrap();
    p.tick();
    assert!(p.sm[0].stalled);
    assert_eq!(p.sm[0].pc, 0);
    assert_eq!(
        lines.pad_drive(31),
        labwired_core::logic_capture::PadDrive::HighZ
    );
    assert!(p.wire_input_edge(31, true, 10));
    assert_eq!(lines.input(31), Some(true));
    p.tick();
    p.tick();
    p.tick();
    assert_eq!(p.read_u32(0x20).unwrap(), 0x8000_0000);
    assert_eq!(p.pin_dir, 0, "receiving does not claim the pad output");
    assert!(!p.wire_input_edge(32, true, 10));
}

#[test]
fn full_rx_fifo_keeps_a_scheduler_producer_live_until_a_read_frees_space() {
    use labwired_core::peripherals::pio::Pio;
    use labwired_core::{sched::EventScheduler, CycleClock};
    let mut p = Pio::new();
    p.attach_cycle_clock(CycleClock::default());
    p.write_u32(0x48, 0x8020).unwrap(); // PUSH BLOCK
    p.write_u32(0xcc, 0).unwrap();
    p.rx_fifo[0].borrow_mut().extend([1, 2, 3, 4]);
    p.sm[0].isr = 5;
    p.write_u32(0, 1).unwrap();
    if p.uses_scheduler() {
        let token = p.take_scheduled_events()[0].1;
        let mut sched = EventScheduler::new();
        let mut b = SystemBus::new();
        assert_eq!(
            p.on_event(token, &mut sched, &mut b).reschedule_delay,
            Some(1)
        );
        assert_eq!(p.read_u32(0x20).unwrap(), 1);
        p.on_event(token, &mut sched, &mut b);
    } else {
        p.tick();
        assert!(p.sm[0].stalled);
        assert_eq!(p.read_u32(0x20).unwrap(), 1);
        p.tick();
    }
    assert_eq!(*p.rx_fifo[0].borrow(), [2, 3, 4, 5]);
}

#[test]
fn malformed_descriptor_routes_fail_and_reset_preserves_valid_routes() {
    let path = root("configs/chips/rp2354b.yaml");
    let original = ChipDescriptor::from_file(&path).unwrap();
    let system: SystemManifest =
        serde_yaml::from_str(&format!("name: dma-route-test\nchip: {}\n", path.display())).unwrap();
    for value in [
        "null",
        "[1,2]",
        "[0,4096,8192]",
        "[4096,4096,8192]",
        "[4097,8192,12288]",
        "[4294967296,8192,12288]",
        "[true,8192,12288]",
    ] {
        let mut chip = original.clone();
        chip.peripherals
            .iter_mut()
            .find(|p| p.id == "dma")
            .unwrap()
            .config
            .insert(
                "pio_dreq_bases".into(),
                serde_yaml::from_str(value).unwrap(),
            );
        assert!(SystemBus::from_config(&chip, &system).is_err(), "{value}");
    }
    let mut chip = original.clone();
    chip.peripherals
        .iter_mut()
        .find(|p| p.id == "dma")
        .unwrap()
        .config
        .remove("pio_dreq_bases");
    assert!(SystemBus::from_config(&chip, &system).is_err());
    let mut d = dma().with_pio_dreq_bases([0x5100_0000, 0x5101_0000, 0x5102_0000]);
    d.on_system_reset();
    setup(&mut d, 0, 1, ctrl(0) & !(63 << 17));
    assert_ne!(d.read_u32(12).unwrap() & (1 << 26), 0);
}
