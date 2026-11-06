//! Exact G0B1 descriptor and TIOL112 digital timing, not a silicon WCET claim.
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::network::timed_uart::{
    cycles_to_ps, ps_to_cycles_ceil, LineFormat, Parity, TimedUartNet,
};
use labwired_core::peripherals::components::tiol112::{
    Tiol112, DRIVER_MAX_DELAY_PS, RECEIVER_MAX_DELAY_PS,
};
use labwired_core::peripherals::stm32g0_clock::{validate_timed_uart_clock, G0Clocks};
use labwired_core::peripherals::uart::Uart;
use labwired_core::Bus;

const HZ: u64 = 64_000_000;
const UART: u64 = 0x40004400;
const RCC: u64 = 0x40021000;
const GPIO: u64 = 0x50000000;
const CR1: u32 = 1 | (1 << 2) | (1 << 3) | (1 << 10) | (1 << 12);

fn bus(config: &str, hz: u64) -> anyhow::Result<SystemBus> {
    let chip = ChipDescriptor::from_file(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../configs/chips/stm32g0b1re.yaml"
    ))?;
    let manifest: SystemManifest = serde_yaml::from_str(&format!(
        "name: g0-tiol112\nchip: ignored\ncpu_hz: {hz}\nexternal_devices:\n- id: phy\n  type: tiol112\n  connection: usart2\n  config: {{{config}}}\n"))?;
    SystemBus::from_config(&chip, &manifest)
}

fn initialize64(bus: &mut SystemBus) {
    bus.write_u32(RCC + 0x0c, 0x30000802).unwrap(); // HSI16/M1*N8/R2, PLLREN
    bus.write_u32(RCC, 0x01000500).unwrap();
    bus.write_u32(RCC + 8, 2).unwrap(); // SW=PLLR, AHB/APB /1
    bus.write_u32(RCC + 0x34, 3).unwrap();
    bus.write_u32(RCC + 0x3c, (1 << 17) | 1).unwrap();
    bus.write_u32(GPIO, (2 << 4) | (2 << 6) | (1 << 16))
        .unwrap(); // PA2/3 AF, PA8 EN output
    bus.write_u32(GPIO + 0x20, (1 << 8) | (1 << 12)).unwrap();
    bus.write_u32(UART + 0x0c, 278).unwrap();
    bus.write_u32(UART, CR1).unwrap();
}

fn service(bus: &mut SystemBus, cycle: u64) {
    bus.set_current_cycle(cycle);
    // These register-level component oracles have no CPU/Machine event loop.
    // Use the existing peripheral-only boundary in either feature mode. The
    // Machine test below separately verifies real scheduler dispatch.
    bus.tick_peripherals_fully_forced();
}

fn phy(bus: &SystemBus) -> Tiol112 {
    let idx = bus.find_peripheral_index_by_name("usart2").unwrap();
    bus.peripherals[idx]
        .dev
        .as_any()
        .unwrap()
        .downcast_ref::<Uart>()
        .unwrap()
        .tiol112()
        .unwrap()
        .clone()
}

fn format() -> LineFormat {
    LineFormat {
        bit_ps: cycles_to_ps(278, HZ),
        data_bits: 8,
        parity: Parity::Even,
        stop_half_bits: 2,
        rx_enabled: true,
        tx_enabled: true,
    }
}

#[test]
fn exact_g0b1_memory_irq_and_64mhz_clock_guard() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let board = SystemManifest::from_file(root.join("configs/systems/nucleo-g0b1re.yaml")).unwrap();
    let chip = ChipDescriptor::from_file(root.join("configs/chips/stm32g0b1re.yaml")).unwrap();
    let board_bus = SystemBus::from_config(&chip, &board).unwrap();
    assert_eq!(board_bus.cpu_hz, 16_000_000, "board reset clock");
    let mut bus = bus("en: PA8", HZ).unwrap();
    assert!(
        validate_timed_uart_clock(&bus, "usart2").is_err(),
        "16MHz reset must not be mislabeled64MHz"
    );
    initialize64(&mut bus);
    let clocks = validate_timed_uart_clock(&bus, "usart2").unwrap();
    assert_eq!(
        clocks,
        G0Clocks {
            sysclk_hz: HZ,
            hclk_hz: HZ,
            pclk_hz: HZ,
            timer_hz: HZ
        }
    );
    bus.write_u32(0x20023ffc, 0xa55aa55a).unwrap();
    assert_eq!(
        bus.read_u32(0x20023ffc).unwrap(),
        0xa55aa55a,
        "top of144KiB SRAM"
    );
    let idx = bus.find_peripheral_index_by_name("usart2").unwrap();
    assert_eq!(bus.peripherals[idx].irq, Some(28));
    bus.write_u32(RCC + 8, 2 | (4 << 12)).unwrap();
    assert!(
        validate_timed_uart_clock(&bus, "usart2").is_err(),
        "APB /2 unsupported by current timed UART must be rejected"
    );
    let divided = G0Clocks::from_registers(
        bus.read_u32(RCC).unwrap(),
        bus.read_u32(RCC + 8).unwrap(),
        bus.read_u32(RCC + 12).unwrap(),
        None,
    )
    .unwrap();
    assert_eq!((divided.pclk_hz, divided.timer_hz), (32_000_000, HZ));
}

#[test]
fn tiol112_truth_table_and_bad_wiring_fail_closed() {
    for tx in [false, true] {
        assert_eq!(Tiol112::cq_drive(false, tx), None);
        assert_eq!(Tiol112::cq_drive(true, tx), Some(!tx));
        assert_eq!(Tiol112::receiver_level(!tx), tx);
    }
    for invalid in [
        "wake: PB0",
        "en: missing",
        "en: PA8, unknown: true",
        "en: PA8, driver_delay_ns: 1201",
        "en: PA8, wake: 1",
    ] {
        assert!(bus(invalid, HZ).is_err(), "must reject {invalid}");
    }
}

#[test]
fn wake_and_fault_are_active_low_and_wake_reaches_real_exti() {
    let mut bus = bus("en: PA8, wake: PB0, nfault: PB1", HZ).unwrap();
    initialize64(&mut bus);
    bus.write_u32(0x50000400, 0).unwrap(); // PB0/1 inputs
    bus.write_u32(0x40021860, 1).unwrap(); // EXTI0 PB0
    bus.write_u32(0x40021804, 1).unwrap();
    bus.write_u32(0x40021880, 1).unwrap();
    assert_eq!(bus.read_u32(0x50000410).unwrap() & 3, 3);
    bus.set_input(Some("phy"), "wake", 1.0).unwrap();
    service(&mut bus, 1);
    assert_eq!(bus.read_u32(0x50000410).unwrap() & 1, 0);
    assert_eq!(
        bus.read_u32(0x40021810).unwrap() & 1,
        1,
        "falling WAKE must pend EXTI0"
    );
    bus.set_input(Some("phy"), "fault", 1.0).unwrap();
    service(&mut bus, 2);
    assert_eq!(bus.read_u32(0x50000410).unwrap() & 2, 0);
    bus.set_input(Some("phy"), "wake", 0.0).unwrap();
    service(&mut bus, 3);
    assert_eq!(bus.read_u32(0x50000410).unwrap() & 3, 1);
}

#[test]
fn en_blocks_tx_and_timed_cq_inverts_with_propagation_and_echo() {
    let mut bus = bus("en: PA8", HZ).unwrap();
    let net = TimedUartNet::new();
    let (_, device, master) = net.add_link(("g0", "usart2", HZ), ("master", "uart", HZ), 0, 0, 1);
    bus.attach_timed_uart_by_id("usart2", device).unwrap();
    initialize64(&mut bus);
    let gate = phy(&bus);
    bus.write_u32(UART + 0x28, 0x35).unwrap();
    service(&mut bus, 4000);
    assert_eq!(gate.disabled_tx_count(), 1);
    assert!(gate.tx_frames().is_empty());
    bus.write_u32(GPIO + 0x18, 1 << 8).unwrap();
    service(&mut bus, 4000 + 6 * 64);
    bus.write_u32(UART + 0x28, 0xa5).unwrap();
    service(&mut bus, 4001 + 6 * 64);
    let frames = gate.tx_frames();
    assert_eq!(frames.len(), 1);
    assert_eq!(
        frames[0].start_ps,
        cycles_to_ps(4001 + 6 * 64, HZ) + DRIVER_MAX_DELAY_PS
    );
    assert_eq!(
        frames[0].cq_level_at(frames[0].start_ps),
        Some(true),
        "UART start low becomes CQhigh"
    );
    assert_eq!(
        frames[0].cq_level_at(frames[0].start_ps + format().bit_ps),
        Some(false),
        "LSB1 becomes CQlow"
    );
    service(&mut bus, 8000);
    let (received, lost) = master.receive_due(8000, &format(), false);
    assert_eq!(lost, 0);
    assert_eq!(received.unwrap().value & 0xff, 0xa5);
    assert_eq!(
        bus.read_u32(UART + 0x24).unwrap() & 0xff,
        0xa5,
        "TI RX echoes CQ unless firmware disables RE"
    );
}

#[test]
fn timing_oracle_uses_final_stop_end_and_first_cq_start_not_tc() {
    let mut bus = bus("en: PA8", HZ).unwrap();
    let net = TimedUartNet::new();
    let (_, device, master) = net.add_link(("g0", "usart2", HZ), ("master", "uart", HZ), 0, 0, 1);
    bus.attach_timed_uart_by_id("usart2", device).unwrap();
    initialize64(&mut bus);
    validate_timed_uart_clock(&bus, "usart2").unwrap();
    let fmt = format();
    let start = 100;
    master.transmit(start, 0x02, fmt); // last master request octet
    let last_stop_end_ps = cycles_to_ps(start, HZ) + fmt.frame_ps();
    let rxne_cycle = ps_to_cycles_ceil(
        cycles_to_ps(start, HZ) + fmt.sample_offset_ps() + RECEIVER_MAX_DELAY_PS,
        HZ,
    );
    service(&mut bus, rxne_cycle);
    assert_ne!(bus.read_u32(UART + 0x1c).unwrap() & (1 << 5), 0);
    assert_eq!(bus.read_u32(UART + 0x24).unwrap() & 0xff, 2);
    // Controlled12us software latency after complete RX stop, independent of
    // frame length. This verifies the measurement, not application execution.
    let response_write = ps_to_cycles_ceil(last_stop_end_ps, HZ) + 12 * 64;
    service(&mut bus, response_write - 6 * 64);
    bus.write_u32(UART, CR1 & !(1 << 2)).unwrap();
    bus.write_u32(GPIO + 0x18, 1 << 8).unwrap();
    service(&mut bus, response_write);
    bus.write_u32(UART + 0x28, 0xa5).unwrap();
    service(&mut bus, response_write + 1);
    let frame = phy(&bus).tx_frames().remove(0);
    let ta_ps = frame.start_ps - last_stop_end_ps;
    assert_eq!(ta_ps, 12_000_000 + 15_625 + DRIVER_MAX_DELAY_PS);
    assert!(ta_ps > fmt.bit_ps && ta_ps < 10 * fmt.bit_ps);
    assert_eq!(frame.stop_end_ps - frame.start_ps, 11 * fmt.bit_ps);
    assert!(
        frame.stop_end_ps - last_stop_end_ps > 10 * fmt.bit_ps,
        "TC is not tA; even one complete reply octet crosses the bound"
    );
}

#[test]
fn no_enable_wait_loses_first_frame_and_configured_guard_passes() {
    let mut bus = bus("en: PA8, enable_delay_ns: 4000", HZ).unwrap();
    let net = TimedUartNet::new();
    let (_, device, _master) = net.add_link(("g0", "usart2", HZ), ("master", "uart", HZ), 0, 0, 1);
    bus.attach_timed_uart_by_id("usart2", device).unwrap();
    initialize64(&mut bus);
    let gate = phy(&bus);
    bus.write_u32(GPIO + 0x18, 1 << 8).unwrap();
    bus.write_u32(UART + 0x28, 0xa5).unwrap();
    service(&mut bus, 1);
    assert_eq!(
        gate.enable_not_ready_count(),
        1,
        "no-wait negative control must lose its clean start"
    );
    assert!(gate.tx_frames().is_empty());
    service(&mut bus, 4000); // first MCU character completed, driver now ready
    bus.write_u32(UART + 0x28, 0x5a).unwrap();
    service(&mut bus, 4001);
    assert_eq!(gate.tx_frames().len(), 1);
}

#[test]
fn disable_delay_truncates_in_flight_cq_and_preserves_time_of_release() {
    let mut bus = bus("en: PA8, enable_delay_ns: 4000, disable_delay_ns: 4000", HZ).unwrap();
    let net = TimedUartNet::new();
    let (_, device, master) = net.add_link(("g0", "usart2", HZ), ("master", "uart", HZ), 0, 0, 1);
    bus.attach_timed_uart_by_id("usart2", device).unwrap();
    initialize64(&mut bus);
    bus.write_u32(GPIO + 0x18, 1 << 8).unwrap();
    service(&mut bus, 6 * 64);
    bus.write_u32(UART + 0x28, 0x00).unwrap();
    service(&mut bus, 6 * 64 + 1);
    service(&mut bus, 8 * 64);
    bus.write_u32(GPIO + 0x18, 1 << 24).unwrap();
    let frame = phy(&bus).tx_frames().remove(0);
    assert_eq!(frame.truncated_ps, Some(12_000_000));
    let (received, _) = master.receive_due(5000, &format(), false);
    assert!(
        received.is_none_or(|r| r.value & 0xff != 0 || r.framing_error || r.parity_error),
        "premature EN release must not deliver a clean0x00"
    );
}

#[test]
fn live_clock_guard_blocks_wrong_timebase_until_firmware_programs_pll() {
    let mut bus = bus("en: high", HZ).unwrap();
    let net = TimedUartNet::new();
    let (_, device, _master) = net.add_link(("g0", "usart2", HZ), ("master", "uart", HZ), 0, 0, 1);
    bus.attach_timed_uart_by_id("usart2", device).unwrap();
    // Configure a real, clocked AF route without changing reset HSI16. If the
    // pins were left disconnected, pad gating would suppress TX before this
    // negative control reached the live clock contract.
    bus.write_u32(RCC + 0x34, 1).unwrap();
    bus.write_u32(GPIO, (2 << 4) | (2 << 6)).unwrap();
    bus.write_u32(GPIO + 0x20, (1 << 8) | (1 << 12)).unwrap();
    bus.write_u32(RCC + 0x3c, 1 << 17).unwrap();
    bus.write_u32(UART + 0x0c, 278).unwrap();
    bus.write_u32(UART, CR1).unwrap();
    bus.write_u32(UART + 0x28, 0xa5).unwrap();
    service(&mut bus, 4000);
    let gate = phy(&bus);
    assert!(gate.clock_error().unwrap().contains("16000000"));
    assert!(gate.tx_frames().is_empty());
    initialize64(&mut bus);
    bus.write_u32(UART + 0x28, 0x5a).unwrap();
    service(&mut bus, 4001);
    assert_eq!(
        gate.tx_frames().len(),
        1,
        "actualPLL64 setup opens the clock gate"
    );
}

#[test]
fn machine_advance_services_tiol112_uart_and_enable_edges() {
    use labwired_core::memory::ProgramImage;
    use labwired_core::system::cortex_m::configure_cortex_m;
    use labwired_core::{Arch, Machine};

    let mut bus = bus("en: PA8", HZ).unwrap();
    let net = TimedUartNet::new();
    let (_, device, master) = net.add_link(("g0", "usart2", HZ), ("master", "uart", HZ), 0, 0, 1);
    bus.attach_timed_uart_by_id("usart2", device).unwrap();
    initialize64(&mut bus);
    let (cpu, _) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.config.peripheral_tick_interval = 1;
    machine.bus.config.peripheral_tick_interval = 1;
    let mut bytes = vec![0; 0x102];
    bytes[..4].copy_from_slice(&0x2002_4000u32.to_le_bytes());
    bytes[4..8].copy_from_slice(&0x0800_0101u32.to_le_bytes());
    bytes[0x100..].copy_from_slice(&[0xfe, 0xe7]); // Thumb B .
    let mut image = ProgramImage::new(0x0800_0101, Arch::Arm);
    image.add_segment(0x0800_0000, bytes);
    machine.load_firmware(&image).unwrap();
    let advance = |machine: &mut Machine<_>, cycles: u64| {
        let target = machine.total_cycles + cycles;
        while machine.total_cycles < target {
            machine.step().unwrap();
        }
    };
    machine.bus.write_u32(GPIO + 0x18, 1 << 8).unwrap();
    advance(&mut machine, 6 * 64);
    machine.bus.write_u32(UART + 0x28, 0xa5).unwrap();
    advance(&mut machine, 4000);
    let gate = phy(&machine.bus);
    assert_eq!(gate.tx_frame_count(), 1, "Machine must service UART events");
    let (got, lost) = master.receive_due(machine.total_cycles, &format(), false);
    assert_eq!(lost, 0);
    assert_eq!(got.unwrap().value, 0xa5);
    assert_eq!(machine.bus.read_u32(UART + 0x24).unwrap() & 0xff, 0xa5);
    machine.bus.write_u32(GPIO + 0x18, 1 << 24).unwrap();
    advance(&mut machine, 10 * 64);
    master.transmit(machine.total_cycles + 64, 0x42, format());
    // This standalone peer plays World’s role: World calls this after peers
    // enqueue wire activity so an idle scheduled receiver gets a deadline.
    machine.timed_uart_sync();
    advance(&mut machine, 4000);
    assert_eq!(machine.bus.read_u32(UART + 0x24).unwrap() & 0xff, 0x42);
    assert!(gate.tx_frames()[0].truncated_ps.is_none());
}
