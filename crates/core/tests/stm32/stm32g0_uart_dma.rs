//! G0 DMA1/DMAMUX and timed 8E1 UART register-level oracles.
//! These exercise actual bus copies. The external-ELF IO-Link gate separately
//! proves the compiled CMSIS/HAL drivers and their C/Q response timing.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::network::timed_uart::{
    cycles_to_ps, LineFormat, Parity, TimedUartNet, TimedUartPort,
};
use labwired_core::Bus;

const HZ: u64 = 64_000_000;
const BIT: u64 = 278;
const FRAME: u64 = BIT * 11;
const UART: u64 = 0x4001_3800;
const DMA: u64 = 0x4002_0000;
const MUX: u64 = 0x4002_0800;
const RCC: u64 = 0x4002_1000;
const RAM: u64 = 0x2000_4000;
const RX: u32 = 1 << 6;
const TX: u32 = 1 << 7;
const CR1: u32 = 1 | (1 << 2) | (1 << 3) | (1 << 10) | (1 << 12);

fn setup() -> (SystemBus, TimedUartPort, LineFormat) {
    let chip = ChipDescriptor::from_file(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../configs/chips/stm32g0b1re.yaml"
    ))
    .unwrap();
    let manifest: SystemManifest =
        serde_yaml::from_str("name: g0-dma-test\nchip: ignored\ncpu_hz: 64000000\n").unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    // The Cortex-M setup attaches the NVIC state to the bus, as a real
    // Machine does. A descriptor-only component bus has no IRQ sink.
    let _ = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    bus.config.peripheral_tick_interval = 1;
    bus.write_u32(RCC + 0x38, 1).unwrap();
    bus.write_u32(RCC + 0x40, 1 << 14).unwrap();
    let net = TimedUartNet::new();
    let (_, device, peer) = net.add_link(("g0", "usart1", HZ), ("peer", "uart", HZ), 0, 0, 1);
    bus.attach_timed_uart_by_id("usart1", device).unwrap();
    bus.write_u32(UART + 0x0c, BIT as u32).unwrap();
    bus.write_u32(UART, CR1).unwrap();
    (
        bus,
        peer,
        LineFormat {
            bit_ps: cycles_to_ps(BIT, HZ),
            data_bits: 8,
            parity: Parity::Even,
            stop_half_bits: 2,
            rx_enabled: true,
            tx_enabled: true,
        },
    )
}

fn channel(bus: &mut SystemBus, number: u32, address: u64, count: u32, flags: u32) {
    let base = DMA + 8 + u64::from(number - 1) * 20;
    bus.write_u32(base, 0).unwrap();
    bus.write_u32(DMA + 4, 1 << (4 * (number - 1))).unwrap();
    bus.write_u32(base + 4, count).unwrap();
    bus.write_u32(
        base + 8,
        (UART + if flags & (1 << 4) == 0 { 0x24 } else { 0x28 }) as u32,
    )
    .unwrap();
    bus.write_u32(base + 12, address as u32).unwrap();
    bus.write_u32(base, 1 | flags).unwrap();
    bus.write_u32(
        MUX + u64::from(number - 1) * 4,
        if flags & (1 << 4) == 0 { 50 } else { 51 },
    )
    .unwrap();
}

fn run(bus: &mut SystemBus, until: u64) {
    while bus.current_cycle() < until {
        bus.set_current_cycle(bus.current_cycle() + 1);
        // No CPU exists in these component tests. A separate Machine test
        // below exercises production event dispatch in both feature builds.
        bus.tick_peripherals_fully_forced();
    }
}

fn receive(bus: &mut SystemBus, peer: &TimedUartPort, fmt: LineFormat, value: u8) {
    let start = bus.current_cycle() + 10;
    peer.transmit(start, u16::from(value), fmt);
    let frame_cycles = labwired_core::network::timed_uart::ps_to_cycles_ceil(fmt.frame_ps(), HZ);
    run(bus, start + frame_cycles + 10);
}

#[test]
fn circular_rx_reloads_count_and_memory_base_with_exact_half_and_full_irqs() {
    let (mut bus, peer, fmt) = setup();
    channel(&mut bus, 1, RAM, 8, (1 << 7) | (1 << 5) | 0xe);
    bus.write_u32(UART + 8, RX).unwrap();
    for n in 0..20u8 {
        receive(&mut bus, &peer, fmt, 0x80 + n);
        assert_eq!(bus.read_u32(DMA + 12).unwrap(), 8 - (u32::from(n + 1) % 8));
        assert_eq!(bus.read_u8(RAM + u64::from(n % 8)).unwrap(), 0x80 + n);
        assert_eq!(bus.read_u32(DMA + 0x10).unwrap(), (UART + 0x24) as u32);
        assert_eq!(bus.read_u32(DMA + 0x14).unwrap(), RAM as u32);
        let flags = bus.read_u32(DMA).unwrap() & 0xf;
        if (n + 1) % 8 == 4 {
            assert_eq!(flags, 5, "half completion must set HTIF+GIF");
            assert_ne!(bus.read_u32(0xe000_e200).unwrap() & (1 << 9), 0);
            bus.write_u32(DMA + 4, 4).unwrap();
            assert_eq!(
                bus.read_u32(DMA).unwrap() & 0xf,
                0,
                "GIF is the OR of remaining flags"
            );
            assert_eq!(bus.read_u32(0xe000_e200).unwrap() & (1 << 9), 0);
        } else if (n + 1) % 8 == 0 {
            assert_eq!(flags, 3, "full completion and circular reload are atomic");
            bus.write_u32(DMA + 4, 1).unwrap();
            assert_eq!(
                bus.read_u32(DMA).unwrap() & 0xf,
                0,
                "CGIF clears the whole group"
            );
        } else {
            assert_eq!(
                flags, 0,
                "HT must not retrigger after being cleared in a lap"
            );
        }
    }
}

#[test]
fn tx_dma_completion_precedes_usart_tc_and_moves_every_byte_through_tdr() {
    let (mut bus, peer, fmt) = setup();
    let data = [0x31, 0x7e, 0xa5, 0xc3];
    for (i, byte) in data.into_iter().enumerate() {
        bus.write_u8(RAM + i as u64, byte).unwrap();
    }
    channel(&mut bus, 2, RAM, data.len() as u32, (1 << 4) | (1 << 7) | 2);
    bus.write_u32(UART + 8, TX).unwrap();
    let mut received = Vec::new();
    let mut dma_tc_cycle = None;
    for cycle in 1..=FRAME * 5 {
        run(&mut bus, cycle);
        let (got, overruns) = peer.receive_due(cycle, &fmt, false);
        assert_eq!(overruns, 0);
        if let Some(item) = got {
            assert!(!item.parity_error && !item.framing_error);
            received.push(item.value as u8);
        }
        if dma_tc_cycle.is_none() && bus.read_u32(DMA + 0x20).unwrap() == 0 {
            dma_tc_cycle = Some(cycle);
            assert_eq!(
                bus.read_u32(UART + 0x1c).unwrap() & (1 << 6),
                0,
                "DMA TC must not fabricate USART TC"
            );
            assert_ne!(bus.read_u32(DMA).unwrap() & (2 << 4), 0);
            assert_ne!(bus.read_u32(0xe000_e200).unwrap() & (1 << 10), 0);
            bus.write_u32(UART + 8, 0).unwrap();
        }
    }
    assert!(dma_tc_cycle.unwrap() < 3 * FRAME);
    assert_eq!(received, data);
    assert_ne!(bus.read_u32(UART + 0x1c).unwrap() & (1 << 6), 0);
}

#[test]
fn request_id_dma_gate_disable_and_rearm_are_real() {
    let (mut bus, peer, fmt) = setup();
    channel(&mut bus, 1, RAM, 2, 1 << 7);
    bus.write_u32(MUX, 49).unwrap();
    bus.write_u32(UART + 8, RX).unwrap();
    receive(&mut bus, &peer, fmt, 0x42);
    assert_eq!(
        bus.read_u32(DMA + 12).unwrap(),
        2,
        "wrong DMAMUX input must not transfer"
    );
    bus.write_u32(MUX, 50).unwrap();
    bus.write_u32(RCC + 0x38, 0).unwrap();
    let until = bus.current_cycle() + 10;
    run(&mut bus, until);
    assert_eq!(
        bus.read_u8(RAM).unwrap(),
        0,
        "AHB clock off prevents the copy"
    );
    bus.write_u32(RCC + 0x38, 1).unwrap();
    assert_eq!(
        bus.read_u32(DMA + 12).unwrap(),
        2,
        "counter retained while clock gated"
    );
    let until = bus.current_cycle() + 10;
    run(&mut bus, until);
    assert_eq!(bus.read_u8(RAM).unwrap(), 0x42);
    bus.write_u32(DMA + 8, 0).unwrap();
    receive(&mut bus, &peer, fmt, 0x53);
    assert_eq!(bus.read_u32(DMA + 12).unwrap(), 1);
    channel(&mut bus, 1, RAM + 16, 1, 1 << 7);
    let until = bus.current_cycle() + 10;
    run(&mut bus, until);
    assert_eq!(bus.read_u32(DMA + 12).unwrap(), 0);
    assert_eq!(
        bus.read_u8(RAM + 16).unwrap(),
        0x53,
        "held RXNE request is answered after rearm"
    );
    assert!(
        bus.write_u32(MUX + 7 * 4, 50).is_err(),
        "DMA2 output must fail explicitly"
    );
}

#[test]
fn failed_bus_copy_sets_te_without_completion_and_recovers_after_rearm() {
    let (mut bus, peer, fmt) = setup();
    channel(&mut bus, 1, 0x7000_0000, 1, (1 << 7) | 0xa);
    bus.write_u32(UART + 8, RX).unwrap();
    receive(&mut bus, &peer, fmt, 0x61);
    assert_eq!(bus.read_u32(DMA).unwrap() & 0xf, 9, "TEIF+GIF, no false TC");
    assert_eq!(bus.read_u32(DMA + 8).unwrap() & 1, 0);
    assert_eq!(
        bus.read_u32(DMA + 12).unwrap(),
        1,
        "failed destination write does not count"
    );
    assert_ne!(bus.read_u32(0xe000_e200).unwrap() & (1 << 9), 0);
    channel(&mut bus, 1, RAM, 1, (1 << 7) | 0xa);
    receive(&mut bus, &peer, fmt, 0x72);
    assert_eq!(bus.read_u8(RAM).unwrap(), 0x72);
    assert_eq!(bus.read_u32(DMA).unwrap() & 0xf, 3);
}

#[test]
fn ddre_drop_mask_and_explicit_error_recovery_match_modern_usart() {
    for ddre in [false, true] {
        for framing in [false, true] {
            let (mut bus, peer, fmt) = setup();
            channel(&mut bus, 1, RAM, 2, 1 << 7);
            bus.write_u32(UART + 8, RX | if ddre { 1 << 13 } else { 0 })
                .unwrap();
            receive(
                &mut bus,
                &peer,
                LineFormat {
                    parity: if framing { Parity::Even } else { Parity::Odd },
                    data_bits: if framing { 9 } else { 8 },
                    ..fmt
                },
                if framing { 0 } else { 0x35 },
            );
            let status = bus.read_u32(UART + 0x1c).unwrap();
            assert_ne!(status & if framing { 2 } else { 1 }, 0);
            assert_eq!(status & (1 << 5) != 0, ddre);
            assert_eq!(
                bus.read_u32(DMA + 12).unwrap(),
                2,
                "corrupt item must not reach RAM"
            );
            assert_ne!(
                bus.read_u32(UART + 8).unwrap() & RX,
                0,
                "hardware does not clear DMAR"
            );
            bus.write_u32(UART + 8, 0).unwrap();
            bus.write_u32(UART + 0x18, 1 << 3).unwrap();
            bus.write_u32(UART + 0x20, 3).unwrap();
            bus.write_u32(UART + 8, RX).unwrap();
            receive(&mut bus, &peer, fmt, 0x46);
            assert_eq!(bus.read_u32(DMA + 12).unwrap(), 1);
            assert_eq!(bus.read_u8(RAM).unwrap(), 0x46);
        }
    }
}

#[test]
fn overrun_retains_old_rdr_and_ovrdis_overwrites_without_ore() {
    for overwrite in [false, true] {
        let (mut bus, peer, fmt) = setup();
        bus.write_u32(UART + 8, RX | if overwrite { 1 << 12 } else { 0 })
            .unwrap();
        receive(&mut bus, &peer, fmt, 0x19);
        receive(&mut bus, &peer, fmt, 0x2a);
        let status = bus.read_u32(UART + 0x1c).unwrap();
        assert_eq!(status & (1 << 3) != 0, !overwrite);
        assert_eq!(
            bus.read_u8(UART + 0x24).unwrap(),
            if overwrite { 0x2a } else { 0x19 }
        );
    }
}

#[test]
fn delayed_uart_service_applies_overwrite_and_dma_error_drop_per_character() {
    for overwrite in [false, true] {
        let (mut bus, peer, fmt) = setup();
        bus.write_u32(UART + 8, RX | if overwrite { 1 << 12 } else { 0 })
            .unwrap();
        peer.transmit(
            10,
            0x35,
            if overwrite {
                fmt
            } else {
                LineFormat {
                    parity: Parity::Odd,
                    ..fmt
                }
            },
        );
        peer.transmit(10 + FRAME, 0x46, fmt);
        bus.set_current_cycle(10 + FRAME * 2);
        bus.tick_peripherals_fully_forced();
        assert_eq!(
            bus.read_u8(UART + 0x24).unwrap(),
            0x46,
            "late service must retain the correct final item"
        );
        let status = bus.read_u32(UART + 0x1c).unwrap();
        assert_eq!(status & (1 << 3), 0);
        assert_eq!(
            status & 1 != 0,
            !overwrite,
            "dropped parity error remains sticky"
        );
    }
}

#[test]
fn machine_dispatches_real_timed_dma_requests_and_node_reset_clears_dma() {
    use labwired_core::memory::ProgramImage;
    use labwired_core::system::cortex_m::configure_cortex_m;
    use labwired_core::{Arch, Machine};
    let (mut bus, peer, fmt) = setup();
    channel(&mut bus, 1, RAM, 4, (1 << 7) | (1 << 5));
    bus.write_u32(UART + 8, RX).unwrap();
    let (cpu, _) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.config.peripheral_tick_interval = 1;
    let mut bytes = vec![0; 0x102];
    bytes[..4].copy_from_slice(&0x2002_4000u32.to_le_bytes());
    bytes[4..8].copy_from_slice(&0x0800_0101u32.to_le_bytes());
    bytes[0x100..].copy_from_slice(&[0xfe, 0xe7]);
    let mut image = ProgramImage::new(0x0800_0101, Arch::Arm);
    image.add_segment(0x0800_0000, bytes);
    machine.load_firmware(&image).unwrap();
    for n in 0..10u8 {
        let start = machine.total_cycles + 64;
        peer.transmit(start, u16::from(0x40 + n), fmt);
        machine.timed_uart_sync();
        while machine.total_cycles < start + FRAME + 20 {
            machine.step().unwrap();
        }
        assert_eq!(
            machine.bus.read_u8(RAM + u64::from(n % 4)).unwrap(),
            0x40 + n,
            "dma={} mux={} UART_ISR={:08x} CR3={:08x} AHBENR={:08x}",
            machine.bus.peripherals[machine.bus.find_peripheral_index_by_name("dma1").unwrap()]
                .dev
                .snapshot(),
            machine.bus.peripherals[machine.bus.find_peripheral_index_by_name("dmamux").unwrap()]
                .dev
                .snapshot(),
            machine.bus.read_u32(UART + 0x1c).unwrap(),
            machine.bus.read_u32(UART + 8).unwrap(),
            machine.bus.read_u32(RCC + 0x38).unwrap()
        );
    }
    let idx = machine.bus.find_peripheral_index_by_name("dma1").unwrap();
    let observed = machine.bus.peripherals[idx].dev.snapshot();
    assert_eq!(observed["observations"][0]["completed_items"], 10);
    assert_eq!(machine.bus.read_u32(DMA + 12).unwrap(), 2);
    machine.bus.peripherals[idx].dev.on_node_reset();
    assert_eq!(machine.bus.read_u32(DMA).unwrap(), 0);
    assert_eq!(machine.bus.read_u32(DMA + 8).unwrap(), 0);
    assert_eq!(machine.bus.read_u32(DMA + 12).unwrap(), 0);
    let mux = machine.bus.find_peripheral_index_by_name("dmamux").unwrap();
    machine.bus.peripherals[mux].dev.on_node_reset();
    assert_eq!(machine.bus.read_u32(MUX).unwrap(), 0);
}
