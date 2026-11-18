// SPDX-License-Identifier: MIT
//! Output compare above ARR is an event at natural overflow, not a missing event.
//! Source: ST RM0386, TIMx_SR CC1IF, and the shared STM32-compatible timer model.
use labwired_core::{peripherals::timer::Timer, Peripheral, DMA_SIGNAL_TIMER_CH1};
fn w(t: &mut Timer, r: u64, v: u32) {
    t.write_u32(r, v).unwrap();
}
fn tick_signals(t: &mut Timer) -> Vec<u32> {
    t.tick().dma_signals.unwrap_or_default()
}

#[test]
fn out_of_range_compare_dma_occurs_at_each_wrap_not_software_update() {
    for channel in 0..4_u32 {
        let mut t = Timer::new();
        w(&mut t, 0x2c, 5);
        w(&mut t, 0x34 + u64::from(channel) * 4, 9);
        w(&mut t, 0x0c, 1 << (9 + channel));
        w(&mut t, 0x14, 1);
        assert!(
            tick_signals(&mut t).is_empty(),
            "UG is not a natural overflow"
        );
        assert_eq!(t.read_u32(0x10).unwrap() & (1 << (1 + channel)), 0);
        w(&mut t, 0x10, 0);
        w(&mut t, 0, 1);
        let hits: Vec<_> = (1..=18)
            .filter_map(|cycle| {
                let r = t.tick();
                assert!(!r.irq);
                r.dma_signals.map(|signals| (cycle, signals))
            })
            .collect();
        let signal = DMA_SIGNAL_TIMER_CH1 + channel;
        assert_eq!(
            hits,
            vec![(6, vec![signal]), (12, vec![signal]), (18, vec![signal])]
        );
    }
}

#[test]
fn out_of_range_compare_irq_latches_at_wrap_while_input_capture_does_not() {
    let mut output = Timer::new();
    w(&mut output, 0x2c, 4);
    w(&mut output, 0x34, 8);
    w(&mut output, 0x0c, 1 << 1);
    w(&mut output, 0, 1);
    for _ in 0..4 {
        output.tick();
    }
    assert_eq!(output.read_u32(0x10).unwrap() & 2, 0);
    output.tick();
    assert_eq!(output.read_u32(0x10).unwrap() & 2, 2);
    assert_eq!(output.irq_line_level(), Some(true));

    let mut input = Timer::new();
    w(&mut input, 0x2c, 4);
    w(&mut input, 0x34, 8);
    w(&mut input, 0x18, 1);
    w(&mut input, 0x0c, (1 << 1) | (1 << 9));
    w(&mut input, 0, 1);
    for _ in 0..20 {
        assert!(tick_signals(&mut input).is_empty());
    }
    assert_eq!(input.read_u32(0x10).unwrap() & 2, 0);
    assert_eq!(input.irq_line_level(), Some(false));
}
