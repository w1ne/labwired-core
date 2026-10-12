use labwired_core::{bus::SystemBus, Bus};
const D: u64 = 0xff1f1000;
const C: u64 = 0xff1f2000;
const U: u64 = 0xff4c0000;
fn bus() -> SystemBus {
    let mut b = SystemBus::new();
    b.cpu_hz = 24_000_000;
    b.configure_armv7_rv1103().unwrap();
    b
}
fn cp(n: u32, m: u32, o: u32) -> u32 {
    (n << 16) | (o << 5) | m
}
fn enable(b: &mut SystemBus, id: u32, group1: bool) {
    if group1 {
        b.write_u32(D + 0x80 + (id / 32 * 4) as u64, 1 << (id % 32))
            .unwrap();
    }
    b.write_u32(D + 0x100 + (id / 32 * 4) as u64, 1 << (id % 32))
        .unwrap();
    b.write_u32(D, 3).unwrap();
    b.write_u32(C + 4, 255).unwrap();
    b.write_u32(C, 3).unwrap();
}
#[test]
fn gic_level_ack_eoi_and_linux_probe() {
    let mut b = bus();
    assert_eq!(b.read_u32(D + 4).unwrap(), 7);
    assert_eq!(b.read_u32(C + 0xfc).unwrap() & 0xff0fff, 0x02043b);
    assert_eq!(b.read_u32(D + 0x800).unwrap(), 0x01010101);
    enable(&mut b, 29, false);
    b.arm_cp15_write(cp(14, 2, 0), false, 1).unwrap();
    b.arm_cp15_write(cp(14, 2, 1), false, 1).unwrap();
    b.set_current_cycle(1);
    assert_eq!(b.arm_irq_fiq_levels(), Some((true, false)));
    assert_eq!(b.read_u32(C + 0xc).unwrap(), 29);
    assert_eq!(b.read_u32(C + 0xc).unwrap(), 1023);
    b.write_u32(C + 0x10, 29).unwrap();
    assert_eq!(b.read_u32(C + 0xc).unwrap(), 29);
    b.arm_cp15_write(cp(14, 2, 1), false, 0).unwrap();
    b.write_u32(C + 0x10, 29).unwrap();
    assert_eq!(b.arm_irq_fiq_levels(), Some((false, false)));
}
#[test]
fn gic_priority_masks_groups_and_active_preemption() {
    let mut b = bus();
    enable(&mut b, 59, true);
    enable(&mut b, 60, false);
    b.write_u8(D + 0x400 + 59, 0x80).unwrap();
    b.write_u8(D + 0x400 + 60, 0x20).unwrap();
    b.write_u32(D + 0x204, (1 << 27) | (1 << 28)).unwrap();
    b.write_u32(C + 4, 0x20).unwrap();
    assert_eq!(b.read_u32(C + 0xc).unwrap(), 1023);
    b.write_u32(C + 4, 255).unwrap();
    b.write_u32(C, 11).unwrap();
    assert_eq!(b.arm_irq_fiq_levels(), Some((true, true)));
    assert_eq!(b.read_u32(C + 0xc).unwrap(), 60);
    assert_eq!(b.read_u32(C + 0xc).unwrap(), 1023);
    b.write_u32(C + 0x10, 60).unwrap();
    assert_eq!(b.read_u32(C + 0xc).unwrap(), 59);
}
#[test]
fn timer_signed_mask_deadline_and_counters() {
    let mut b = bus();
    b.cpu_hz = 48_000_000;
    b.set_current_cycle(20);
    assert_eq!(b.arm_cp15_read(cp(14, 0, 0), false).unwrap(), 24_000_000);
    assert_eq!(b.arm_cp15_read(14, true).unwrap(), 10);
    b.arm_cp15_write(cp(14, 2, 0), false, 3).unwrap();
    b.arm_cp15_write(cp(14, 2, 1), false, 1).unwrap();
    assert_eq!(b.next_cpu_interrupt_deadline_cycle(), Some(26));
    b.set_current_cycle(26);
    assert_eq!(b.arm_cp15_read(cp(14, 2, 1), false).unwrap(), 5);
    b.arm_cp15_write(cp(14, 2, 1), false, 3).unwrap();
    assert_eq!(b.next_cpu_interrupt_deadline_cycle(), None);
    b.arm_cp15_write(cp(14, 2, 0), false, u32::MAX as u64)
        .unwrap();
    assert_eq!(b.arm_cp15_read(cp(14, 2, 1), false).unwrap(), 7);
    b.arm_cp15_write(cp(14, 2, 1), false, 0).unwrap();
    assert_eq!(b.arm_cp15_read(cp(14, 2, 1), false).unwrap(), 0);
    assert!(b.arm_cp15_read(0xdead, false).is_err());
}
#[test]
fn virtual_timer_tval_crosses_offset_underflow_without_losing_deadline() {
    use labwired_core::peripherals::armv7::timer::ArchTimer;
    for hz in [24_000_000, 48_000_000] {
        let mut timer = ArchTimer::default();
        timer.synchronize(0, hz);
        timer.write(0x4e, true, 1).unwrap();
        assert_eq!(timer.read(0x1e, true).unwrap(), u64::MAX);
        timer.write(cp(14, 3, 0), false, 1).unwrap();
        timer.write(cp(14, 3, 1), false, 1).unwrap();
        let due = hz / 24_000_000;
        assert_eq!(timer.next_deadline(), Some(due));
        assert_eq!(timer.levels(), (false, false));
        timer.synchronize(due, hz);
        assert_eq!(timer.read(0x1e, true).unwrap(), 0);
        assert_eq!(timer.read(cp(14, 3, 0), false).unwrap(), 0);
        assert_eq!(timer.read(cp(14, 3, 1), false).unwrap(), 5);
        assert_eq!(timer.levels(), (false, true));
        assert_eq!(timer.next_deadline(), Some(due));
        timer.write(cp(14, 3, 1), false, 3).unwrap();
        assert_eq!(timer.levels(), (false, false));
        timer.write(cp(14, 3, 0), false, 4).unwrap();
        timer.write(0x4e, true, 3).unwrap();
        timer.write(cp(14, 3, 1), false, 1).unwrap();
        assert_eq!(timer.read(0x3e, true).unwrap(), 4);
        assert_eq!(timer.read(0x1e, true).unwrap(), u64::MAX - 1);
        assert_eq!(timer.next_deadline(), Some(7 * due));
        timer.synchronize(6 * due, hz);
        assert_eq!(timer.levels(), (false, false));
        timer.synchronize(7 * due, hz);
        assert_eq!(timer.read(0x1e, true).unwrap(), 4);
        assert_eq!(timer.levels(), (false, true));
    }
}
#[test]
fn virtual_timer_is_27_never_30_and_wide_compare_bounded() {
    let mut b = bus();
    enable(&mut b, 27, false);
    enable(&mut b, 30, false);
    b.set_current_cycle(5);
    b.arm_cp15_write(0x4e, true, 5).unwrap();
    b.arm_cp15_write(0x3e, true, 7).unwrap();
    b.arm_cp15_write(cp(14, 3, 1), false, 1).unwrap();
    assert_eq!(b.next_cpu_interrupt_deadline_cycle(), Some(12));
    b.set_current_cycle(12);
    assert_eq!(b.arm_cp15_read(0x1e, true).unwrap(), 7);
    assert_eq!(b.read_u32(C + 0xc).unwrap(), 27);
    b.arm_cp15_write(0x3e, true, u64::MAX).unwrap();
    assert_eq!(b.next_cpu_interrupt_deadline_cycle(), None);
}
#[test]
fn uart_dlab_fifo_widths_interrupts_and_modem_loopback() {
    let mut b = bus();
    enable(&mut b, 59, true);
    let sink = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
    assert!(b.attach_uart_tx_sink_named("uart2", sink.clone(), false));
    b.write_u32(U + 12, 0x80).unwrap();
    b.write_u32(U, 42).unwrap();
    b.write_u16(U + 4, 1).unwrap();
    assert_eq!(b.read_u8(U).unwrap(), 42);
    assert!(sink.lock().unwrap().is_empty());
    b.write_u8(U + 12, 3).unwrap();
    b.write_u32(U, 65).unwrap();
    assert_eq!(*sink.lock().unwrap(), b"A");
    assert_eq!(b.read_u32(U + 0x7c).unwrap(), 6);
    b.write_u32(U + 16, 0x1a).unwrap();
    assert_eq!(b.read_u32(U + 24).unwrap() & 0xf0, 0x90);
    b.write_u32(U + 4, 1).unwrap();
    b.write_u32(U, 66).unwrap();
    assert_eq!(b.read_u32(U + 8).unwrap() & 15, 4);
    assert_eq!(b.arm_irq_fiq_levels(), Some((true, false)));
    assert_eq!(b.read_u32(U).unwrap(), 66);
    assert_eq!(b.read_u32(U + 20).unwrap(), 0x60);
    assert_eq!(b.arm_irq_fiq_levels(), Some((false, false)));
    b.write_u32(U, 67).unwrap();
    b.write_u32(U + 8, 3).unwrap();
    assert_eq!(b.read_u32(U + 8).unwrap(), 0xc1);
    assert!(b.read_u32(U + 1).is_err());
    assert!(b.read_u64(U).is_err());
    assert_eq!(b.read_u32(U + 0xc0).unwrap(), 0);
    assert_eq!(b.read_u32(U + 0xf4).unwrap(), 0);
}
#[test]
fn default_bus_is_inert_and_fabric_clone_gets_distinct_identity() {
    let b = SystemBus::new();
    assert_eq!(b.arm_code_identity(), None);
    assert_eq!(b.arm_irq_fiq_levels(), None);
    let b = bus();
    let f = b.irq_fabric.clone();
    assert_ne!(
        b.irq_fabric.armv7.as_ref().unwrap().code_identity(),
        f.armv7.as_ref().unwrap().code_identity()
    );
}
#[test]
fn timer_negative_at_zero_counter_and_tval_preserves_signed_value() {
    let mut b = bus();
    b.arm_cp15_write(cp(14, 2, 0), false, u32::MAX as u64)
        .unwrap();
    b.arm_cp15_write(cp(14, 2, 1), false, 5).unwrap();
    assert_eq!(
        b.arm_cp15_read(cp(14, 2, 0), false).unwrap(),
        u32::MAX as u64
    );
    assert_eq!(b.arm_cp15_read(cp(14, 2, 1), false).unwrap(), 5);
    b.arm_cp15_write(0x2e, true, u64::MAX).unwrap();
    assert_eq!(b.next_cpu_interrupt_deadline_cycle(), Some(u64::MAX));
    b.set_current_cycle(u64::MAX);
    assert_eq!(b.arm_cp15_read(14, true).unwrap(), u64::MAX);
    assert_eq!(b.arm_cp15_read(cp(14, 2, 1), false).unwrap(), 5);
}
#[test]
fn gic_sgi_edge_clear_pending_and_configuration_widths() {
    let mut b = bus();
    enable(&mut b, 5, false);
    b.write_u32(D + 0xf00, (2 << 24) | 5).unwrap();
    assert_eq!(b.read_u32(C + 0x18).unwrap(), 5);
    assert_eq!(b.read_u32(C + 0xc).unwrap(), 5);
    b.write_u32(C + 0x10, 5).unwrap();
    assert_eq!(b.read_u32(C + 0x18).unwrap(), 1023);
    b.write_u32(D + 0xf00, (1 << 24) | 5).unwrap();
    assert_eq!(b.read_u32(C + 0x18).unwrap(), 1023);
    enable(&mut b, 59, true);
    b.write_u32(D + 0xc0c, 2 << 22).unwrap();
    assert_eq!(b.read_u32(D + 0xc0c).unwrap(), 2 << 22);
    b.write_u16(D + 0x400 + 58, 0x1234).unwrap();
    assert_eq!(b.read_u16(D + 0x400 + 58).unwrap(), 0x1234);
    assert!(b.read_u16(C + 0xc).is_err());
}
#[test]
fn explicit_startup_reductions_accept_only_known_linux_requests() {
    let mut b = bus();
    for (k, v) in [
        (0x10020, 0x40),
        (0xa0002, 0xff0a81a8),
        (0xa0022, 0x40e040e0),
    ] {
        b.arm_cp15_write(k, false, v).unwrap();
        assert!(b.arm_cp15_read(k, false).is_err());
        assert!(b.arm_cp15_write(k, false, 123).is_err());
    }
    assert!(b.arm_cp15_write(0x12345, false, 0).is_err());
}
#[cfg(feature = "qemu-arm-system")]
mod machine {
    use super::*;
    use labwired_core::{cpu::CortexA7System, AdvanceRequest, Cpu, Machine, SimulationObserver};
    use std::num::NonZeroU32;
    use std::sync::{Arc, Mutex};
    #[derive(Debug)]
    struct Trace(
        labwired_core::cycle_clock::CycleClock,
        Mutex<Vec<(u64, u32, u32)>>,
    );
    impl SimulationObserver for Trace {
        fn on_step_start(&self, pc: u32, op: u32) {
            self.1.lock().unwrap().push((self.0.now(), pc, op));
        }
    }
    fn fixture(wfi: bool, masked: bool, deadline: u64) -> (Machine<CortexA7System>, Arc<Trace>) {
        let mut b = SystemBus::new();
        b.peripherals.clear();
        b.cpu_hz = 24_000_000;
        b.configure_armv7_rv1103().unwrap();
        b.ram = labwired_core::memory::LinearMemory::new(0x10000, 0);
        let code = if wfi {
            [0xe320f003, 0xe3a0002a, 0xeafffffe]
        } else {
            [0xe2800001, 0xeafffffd, 0xeafffffe]
        };
        for (i, w) in code.iter().enumerate() {
            b.write_u32(0x100 + i as u64 * 4, *w).unwrap();
        }
        b.write_u32(0x18, 0xe3a0602a).unwrap();
        b.write_u32(0x1c, 0xeafffffe).unwrap();
        enable(&mut b, 29, false);
        b.arm_cp15_write(0x2e, true, deadline).unwrap();
        b.arm_cp15_write(cp(14, 2, 1), false, 1).unwrap();
        let mut c = CortexA7System::new().unwrap();
        c.executable_range(0, 0x10000).unwrap();
        c.set_pc(0x100);
        c.set_register(16, if masked { 0xd3 } else { 0x13 });
        let t = Arc::new(Trace(b.cycle_clock.clone(), Mutex::new(vec![])));
        let mut m = Machine::new(c, b);
        m.config.peripheral_tick_interval = 128;
        m.config.idle_fast_forward_enabled = true;
        m.add_observer(t.clone());
        (m, t)
    }
    #[test]
    fn machine_wfi_wakes_at_actual_timer_cycle_without_fabricated_retirement() {
        for masked in [false, true] {
            let (mut m, t) = fixture(true, masked, 20);
            let report = m
                .advance(AdvanceRequest::run(None).with_cycle_limit(22))
                .unwrap();
            assert_eq!(report.primary_steps, 3);
            assert_eq!(m.total_cycles, 22);
            assert_eq!(m.idle_fast_forward_cycles_skipped, 19);
            let trace = t.1.lock().unwrap();
            assert_eq!(trace.len(), 3);
            assert_eq!(trace[1].0, 20);
            assert_eq!(trace[1].1, if masked { 0x104 } else { 0x18 });
            assert_eq!(m.cpu.get_register(if masked { 0 } else { 6 }), 42);
        }
    }
    #[test]
    fn machine_virtual_timer_wfi_wakes_across_offset_underflow() {
        for masked in [false, true] {
            let (mut m, t) = fixture(true, masked, 1000);
            m.bus.arm_cp15_write(cp(14, 2, 1), false, 0).unwrap();
            enable(&mut m.bus, 27, false);
            m.bus.arm_cp15_write(0x4e, true, 20).unwrap();
            m.bus.arm_cp15_write(cp(14, 3, 0), false, 25).unwrap();
            m.bus.arm_cp15_write(cp(14, 3, 1), false, 1).unwrap();
            assert_eq!(m.bus.next_cpu_interrupt_deadline_cycle(), Some(25));
            let report = m
                .advance(AdvanceRequest::run(None).with_cycle_limit(27))
                .unwrap();
            assert_eq!(report.primary_steps, 3);
            assert_eq!(m.total_cycles, 27);
            assert_eq!(m.idle_fast_forward_cycles_skipped, 24);
            let trace = t.1.lock().unwrap();
            assert_eq!(trace.len(), 3);
            assert_eq!(trace[1].0, 25);
            assert_eq!(trace[1].1, if masked { 0x104 } else { 0x18 });
            assert_eq!(m.cpu.get_register(if masked { 0 } else { 6 }), 42);
            assert_eq!(m.bus.arm_cp15_read(cp(14, 3, 1), false).unwrap(), 5);
            assert_eq!(m.bus.read_u32(C + 0xc).unwrap(), 27);
        }
    }
    #[test]
    fn machine_single_and_batch_timer_edge_are_identical() {
        for wfi in [false, true] {
            for masked in [false, true] {
                let (mut one, a) = fixture(wfi, masked, 7);
                let (mut batch, b) = fixture(wfi, masked, 7);
                let request = AdvanceRequest::run(Some(16)).with_cycle_limit(40);
                one.advance(request.with_batch_cap(NonZeroU32::new(1).unwrap()))
                    .unwrap();
                batch
                    .advance(request.with_batch_cap(NonZeroU32::new(64).unwrap()))
                    .unwrap();
                assert_eq!(*a.1.lock().unwrap(), *b.1.lock().unwrap());
                assert_eq!(one.total_cycles, batch.total_cycles);
                assert_eq!(one.cpu.get_register(0), batch.cpu.get_register(0));
                assert_eq!(one.cpu.get_register(6), batch.cpu.get_register(6));
                assert_eq!(one.bus.arm_irq_fiq_levels(), batch.bus.arm_irq_fiq_levels());
            }
        }
    }
    struct Peer(std::collections::VecDeque<u8>);
    impl labwired_core::peripherals::uart::UartStreamDevice for Peer {
        fn poll(&mut self, _us: u32) -> Option<u8> {
            self.0.pop_front()
        }
        fn on_tx_byte(&mut self, byte: u8) {
            self.0.push_back(byte.wrapping_add(1));
        }
        fn carries_protocol_octets(&self) -> bool {
            true
        }
        fn device_id(&self) -> Option<&str> {
            Some("peer")
        }
    }
    #[test]
    fn machine_uart_peer_uses_existing_time_drive_and_detaches_console() {
        let (mut m, _) = fixture(false, true, 1000);
        let sink = Arc::new(Mutex::new(vec![]));
        assert!(m
            .bus
            .attach_uart_tx_sink_named("uart2", sink.clone(), false));
        m.bus
            .attach_uart_stream_by_id("uart2", Box::new(Peer(std::collections::VecDeque::new())))
            .unwrap();
        m.bus.check_uart_peer("uart2", "peer").unwrap();
        m.config.peripheral_tick_interval = 24;
        m.bus.write_u32(U, 65).unwrap();
        m.advance(AdvanceRequest::run(None).with_cycle_limit(48))
            .unwrap();
        assert!(sink.lock().unwrap().is_empty());
        assert_eq!(m.bus.read_u32(U).unwrap(), 66);
    }
    struct Burst(std::collections::VecDeque<u8>);
    impl labwired_core::peripherals::uart::UartStreamDevice for Burst {
        fn poll(&mut self, _us: u32) -> Option<u8> {
            self.0.pop_front()
        }
        fn max_bytes_per_tick(&self) -> usize {
            16
        }
    }
    #[test]
    fn machine_uart_peer_overrun_uses_same_bounded_rx_fifo() {
        let (mut m, _) = fixture(false, true, 1000);
        m.config.peripheral_tick_interval = 24;
        m.bus
            .attach_uart_stream_by_id("uart2", Box::new(Burst((0..4).collect())))
            .unwrap();
        m.advance(AdvanceRequest::run(None).with_cycle_limit(48))
            .unwrap();
        assert_eq!(m.bus.read_u32(U + 20).unwrap(), 0x63);
        assert_eq!(m.bus.read_u32(U).unwrap(), 0);
        assert_eq!(m.bus.read_u32(U + 20).unwrap(), 0x60);
    }
    #[test]
    fn machine_timer_programming_ends_batch_before_due_irq() {
        let (mut one, a) = fixture(false, false, 1000);
        let (mut batch, b) = fixture(false, false, 1000);
        for m in [&mut one, &mut batch] {
            m.bus.write_u32(0x100, 0xee0e0f12).unwrap();
            m.bus.write_u32(0x104, 0xe3a00063).unwrap();
            m.cpu.set_register(0, 2);
        }
        let request = AdvanceRequest::run(Some(4));
        one.advance(request.with_batch_cap(NonZeroU32::new(1).unwrap()))
            .unwrap();
        batch
            .advance(request.with_batch_cap(NonZeroU32::new(64).unwrap()))
            .unwrap();
        assert_eq!(*a.1.lock().unwrap(), *b.1.lock().unwrap());
        assert_eq!(a.1.lock().unwrap()[2], (2, 0x18, 0xe3a0602a));
        assert_eq!(batch.cpu.get_register(0), 99);
    }
    #[test]
    fn machine_wait_without_reachable_deadline_or_idle_permission_has_zero_extra_retirement() {
        for no_timer in [false, true] {
            let (mut m, t) = fixture(true, true, 20);
            if no_timer {
                m.bus.arm_cp15_write(cp(14, 2, 1), false, 0).unwrap();
            } else {
                m.config.idle_fast_forward_enabled = false;
            }
            let report = m.advance(AdvanceRequest::run(Some(10))).unwrap();
            assert_eq!(report.stop, labwired_core::AdvanceStop::NoProgress);
            assert_eq!(report.primary_steps, 1);
            assert_eq!(m.total_cycles, 1);
            assert_eq!(t.1.lock().unwrap().len(), 1);
        }
    }
    #[test]
    fn real_system_bus_identity_switch_invalidates_native_code() {
        let (mut first, _) = fixture(false, true, 1000);
        let (mut second, _) = fixture(false, true, 1000);
        assert_ne!(
            first.bus.arm_code_identity(),
            second.bus.arm_code_identity()
        );
        first.bus.write_u32(0x100, 0xe3a00001).unwrap();
        second.bus.write_u32(0x100, 0xe3a00002).unwrap();
        first
            .cpu
            .step_retired(
                &mut first.bus,
                &[],
                &labwired_core::SimulationConfig::default(),
            )
            .unwrap();
        assert_eq!(first.cpu.get_register(0), 1);
        first.cpu.set_pc(0x100);
        first
            .cpu
            .step_retired(
                &mut second.bus,
                &[],
                &labwired_core::SimulationConfig::default(),
            )
            .unwrap();
        assert_eq!(first.cpu.get_register(0), 2);
    }
    #[test]
    fn machine_native_startup_reductions_keep_upstream_reads_zero() {
        let (mut m, _) = fixture(false, true, 100);
        let codes = [0xee110f30, 0xee0a1f12, 0xee1a2f12];
        for (i, w) in codes.iter().enumerate() {
            m.bus.write_u32(0x100 + i as u64 * 4, *w).unwrap();
        }
        m.cpu.set_register(1, 0xff0a81a8);
        m.advance(AdvanceRequest::run(Some(3))).unwrap();
        assert_eq!(m.cpu.get_register(0), 0);
        assert_eq!(m.cpu.get_register(2), 0);
    }
}
#[test]
fn named_and_all_uart_rx_attachment_deliver_bounded_fifo_irq() {
    let mut b = bus();
    enable(&mut b, 59, true);
    b.write_u32(U + 8, 1).unwrap();
    b.write_u32(U + 4, 5).unwrap();
    let rx = b.attach_uart_rx_source_named("uart2").unwrap();
    assert!(b
        .attach_uart_rx_source()
        .iter()
        .any(|s| std::sync::Arc::ptr_eq(s, &rx)));
    rx.lock().unwrap().extend(0..32);
    assert_eq!(b.arm_irq_fiq_levels(), Some((true, false)));
    assert_eq!(rx.lock().unwrap().len(), 16);
    assert!(rx.lock().unwrap().capacity() <= 16);
    assert_eq!(b.read_u32(U + 0x7c).unwrap(), 30);
    assert_eq!(b.read_u32(U + 20).unwrap(), 0x63);
    assert_eq!(b.read_u32(U + 8).unwrap() & 15, 4);
    for i in 0..16 {
        assert_eq!(b.read_u32(U).unwrap(), i);
    }
    assert_eq!(b.arm_irq_fiq_levels(), Some((false, false)));
}
#[test]
fn timer_wide_compare_extends_current_counter_epoch_without_overflow() {
    let mut b = bus();
    b.cpu_hz = 1;
    b.set_current_cycle(u64::MAX - 1);
    let now = b.arm_cp15_read(14, true).unwrap();
    b.arm_cp15_write(0x2e, true, now + 24_000_000).unwrap();
    b.arm_cp15_write(cp(14, 2, 1), false, 1).unwrap();
    assert_eq!(b.arm_cp15_read(cp(14, 2, 1), false).unwrap(), 1);
    assert_eq!(b.next_cpu_interrupt_deadline_cycle(), Some(u64::MAX));
    b.set_current_cycle(u64::MAX);
    assert_eq!(b.arm_cp15_read(cp(14, 2, 1), false).unwrap(), 5);
}
#[test]
fn cntkctl_retains_supported_controls_and_refuses_event_stream() {
    let mut b = bus();
    b.arm_cp15_write(cp(14, 1, 0), false, 0x3fb).unwrap();
    assert_eq!(b.arm_cp15_read(cp(14, 1, 0), false).unwrap(), 0x3fb);
    assert!(b.arm_cp15_write(cp(14, 1, 0), false, 4).is_err());
    assert_eq!(b.arm_cp15_read(cp(14, 1, 0), false).unwrap(), 0x3fb);
}
#[test]
fn gic_sgis_are_permanently_enabled_and_sgir_needs_no_enable_write() {
    let mut b = bus();
    assert_eq!(b.read_u32(D + 0x100).unwrap(), 0xffff);
    assert_eq!(b.read_u32(D + 0x180).unwrap(), 0xffff);
    b.write_u32(D + 0x180, u32::MAX).unwrap();
    assert_eq!(b.read_u32(D + 0x100).unwrap(), 0xffff);
    b.write_u32(D + 0x100, 0).unwrap();
    assert_eq!(b.read_u32(D + 0x180).unwrap(), 0xffff);
    b.write_u32(D, 1).unwrap();
    b.write_u32(C, 1).unwrap();
    b.write_u32(C + 4, 255).unwrap();
    b.write_u32(D + 0xf00, (2 << 24) | 3).unwrap();
    assert_eq!(b.arm_irq_fiq_levels(), Some((true, false)));
    assert_eq!(b.read_u32(C + 0xc).unwrap(), 3);
    b.write_u32(C + 0x10, 3).unwrap();
    assert_eq!(b.read_u32(C + 0xc).unwrap(), 1023);
}
#[test]
fn gic_sgi_pending_sources_exist_only_for_cpu_zero() {
    let mut b = bus();
    enable(&mut b, 3, false);
    b.write_u32(D + 0xf20, 0xfe000000).unwrap();
    assert_eq!(b.read_u32(D + 0xf20).unwrap(), 0);
    assert_eq!(b.read_u32(C + 0x18).unwrap(), 1023);
    b.write_u32(D + 0xf20, 0x01000000).unwrap();
    b.write_u32(D + 0xf10, 0xfe000000).unwrap();
    assert_eq!(b.read_u32(D + 0xf10).unwrap(), 0x01000000);
    assert_eq!(b.read_u32(C + 0x18).unwrap(), 3);
    b.write_u32(D + 0xf10, 0x01000000).unwrap();
    assert_eq!(b.read_u32(D + 0xf20).unwrap(), 0);
}
#[test]
fn gic_sgi_pending_registers_and_enable_clear_are_observable() {
    let mut b = bus();
    enable(&mut b, 3, false);
    b.write_u32(D + 0xf20, 0x01000000).unwrap();
    assert_eq!(b.read_u32(D + 0xf20).unwrap(), 0x01000000);
    assert_eq!(b.read_u32(D + 0xf10).unwrap(), 0x01000000);
    b.write_u32(D + 0x180, 8).unwrap();
    assert_eq!(b.read_u32(C + 0x18).unwrap(), 3);
    b.write_u32(D + 0x100, 8).unwrap();
    assert_eq!(b.read_u32(C + 0x18).unwrap(), 3);
    b.write_u32(D + 0xf10, 0x01000000).unwrap();
    assert_eq!(b.read_u32(C + 0x18).unwrap(), 1023);
    enable(&mut b, 31, false);
    b.write_u32(D + 0x200, 1 << 31).unwrap();
    assert_eq!(b.read_u32(C + 0x18).unwrap(), 31);
    b.write_u32(D + 0x180, 1 << 31).unwrap();
    assert_eq!(b.read_u32(C + 0x18).unwrap(), 1023);
    b.write_u32(D + 0x100, 1 << 31).unwrap();
    assert_eq!(b.read_u32(C + 0x18).unwrap(), 31);
}
#[test]
fn uart_thre_latch_and_unknown_accesses() {
    let mut b = bus();
    enable(&mut b, 59, true);
    b.write_u32(U + 4, 2).unwrap();
    assert_eq!(b.arm_irq_fiq_levels(), Some((true, false)));
    assert_eq!(b.read_u32(U + 8).unwrap(), 2);
    assert_eq!(b.arm_irq_fiq_levels(), Some((false, false)));
    b.write_u32(U, 65).unwrap();
    assert_eq!(b.read_u32(U + 8).unwrap(), 2);
    assert_eq!(b.read_u32(U + 8).unwrap(), 1);
    assert!(b.read_u32(U + 0x30).is_err());
    assert!(b.write_u32(U + 0xc0, 256).is_err());
}
#[test]
fn unrepresentable_virtual_deadline_does_not_hide_valid_physical_deadline() {
    let mut b = bus();
    b.cpu_hz = u64::MAX;
    b.set_current_cycle(u64::MAX - 1);
    b.arm_cp15_write(0x2e, true, 24_000_000).unwrap();
    b.arm_cp15_write(cp(14, 2, 1), false, 1).unwrap();
    b.arm_cp15_write(0x4e, true, 3).unwrap();
    b.arm_cp15_write(0x3e, true, u64::MAX).unwrap();
    b.arm_cp15_write(cp(14, 3, 1), false, 1).unwrap();
    assert_eq!(b.next_cpu_interrupt_deadline_cycle(), Some(u64::MAX));
}
#[test]
fn linux_dw_optional_capability_probes_observe_absent_hardware() {
    let mut b = bus();
    b.write_u32(U + 0xb4, 1).unwrap();
    assert_eq!(b.read_u32(U + 0xb4).unwrap(), 0);
    b.write_u32(U + 0xb4, 0).unwrap();
    let old = b.read_u32(U + 0xc0).unwrap();
    b.write_u32(U + 0xc0, u32::MAX).unwrap();
    assert_eq!(b.read_u32(U + 0xc0).unwrap(), 0);
    b.write_u32(U + 0xc0, old).unwrap();
    assert!(b.write_u32(U + 0xb0, 1).is_err());
    assert!(b.write_u32(U + 0xc0, 1).is_err());
}
#[test]
fn gic_bitmap_mutators_cross_all_word_boundaries() {
    for id in [31u32, 32, 63, 64, 95, 127, 128, 191, 192, 255] {
        let mut b = bus();
        enable(&mut b, id, true);
        let word = (id / 32 * 4) as u64;
        let bit = 1u32 << (id % 32);
        b.write_u8(D + 0x400 + id as u64, 0x40).unwrap();
        b.irq_fabric
            .armv7
            .as_ref()
            .unwrap()
            .gic
            .lock()
            .unwrap()
            .set_level(id as usize, true);
        assert_eq!(b.read_u32(D + 0x200 + word).unwrap() & bit, bit);
        assert_eq!(b.arm_irq_fiq_levels(), Some((true, false)));
        assert_eq!(b.read_u32(C + 0xc).unwrap(), id);
        assert_eq!(b.read_u32(D + 0x300 + word).unwrap() & bit, bit);
        assert_eq!(b.read_u32(C + 0x14).unwrap(), 0x40);
        b.write_u32(D + 0x380 + word, bit).unwrap();
        assert_eq!(b.read_u32(D + 0x300 + word).unwrap() & bit, 0);
        b.write_u32(D + 0x300 + word, bit).unwrap();
        assert_eq!(b.read_u32(C + 0xc).unwrap(), 1023);
        b.write_u32(C + 0x10, id).unwrap();
        b.irq_fabric
            .armv7
            .as_ref()
            .unwrap()
            .gic
            .lock()
            .unwrap()
            .set_level(id as usize, false);
        b.write_u32(D + 0x200 + word, bit).unwrap();
        assert_eq!(b.read_u32(C + 0x18).unwrap(), id);
        b.write_u32(D + 0x280 + word, bit).unwrap();
        assert_eq!(b.read_u32(C + 0x18).unwrap(), 1023);
        b.write_u32(D + 0x180 + word, bit).unwrap();
        assert_eq!(b.read_u32(D + 0x100 + word).unwrap() & bit, 0);
    }
}
#[test]
#[ignore = "explicit performance measurement, no wall-clock pass threshold"]
fn gic_level_sampling_benchmark() {
    let mut g = labwired_core::peripherals::armv7::gic::Gic::default();
    for enabled in [false, true] {
        if enabled {
            g.dist_write(0, 4, 3).unwrap();
            g.cpu_write(0, 4, 3).unwrap();
            g.cpu_write(4, 4, 255).unwrap();
        }
        let start = std::time::Instant::now();
        for _ in 0..1_000_000 {
            std::hint::black_box(g.levels());
        }
        println!(
            "GIC 1M level samples enabled={enabled}: {:?}",
            start.elapsed()
        );
    }
}
