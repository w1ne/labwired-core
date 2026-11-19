//! The original model is preserved verbatim as a differential oracle.
use labwired_core::bus::{BusResidentDevice, DevicePins};
use labwired_core::sim_input::SimInput;
#[path = "../common/rotary.rs"]
mod fixture;
#[allow(dead_code)]
#[path = "../common/rotary_oracle.rs"]
mod oracle;
#[derive(Default)]
struct Pads {
    drives: Vec<(bool, u64, u8, bool)>,
}
impl DevicePins for Pads {
    fn output_bit(&self, _: u64, _: u8) -> Option<bool> {
        None
    }
    fn drive_input_bit(&mut self, a: u64, b: u8, v: bool) -> bool {
        self.drives.push((true, a, b, v));
        true
    }
    fn drive_idr_bit(&mut self, a: u64, b: u8, v: bool) {
        self.drives.push((false, a, b, v));
    }
}
#[test]
fn exact_phases_drives_and_timing_match_original() {
    for hz in [
        0, 1, 499, 32_768, 999_999, 1_000_000, 1_500_001, 8_000_000, 78_123_457,
    ] {
        let step = ((2000u128 * hz.max(1) as u128) / 1_000_000).max(1) as u64;
        let mut old = oracle::RotaryEncoder::new("knob".into(), 0x10, 3, 0x20, 4, hz);
        let mut new = fixture::rotary("knob", (0x10, 3), (0x20, 4), hz);
        let mut time = 0;
        let mut a = Pads::default();
        let mut b = Pads::default();
        for target in [0.0, 2.6, 2.4, 2.49, -2.5, -2.51, 0.5, -0.5, 1000.0, -1000.0] {
            old.set_input("position", target).unwrap();
            new.set_input("position", target).unwrap();
            // A delayed first service must anchor without consuming stale elapsed time.
            time += 17 * step + 7;
            for delta in [0, step - 1, 1, step, 3 * step, 8001 * step] {
                time += delta;
                BusResidentDevice::service(&mut old, &mut a, time);
                new.service(&mut b, time);
                assert_eq!(a.drives, b.drives, "hz={hz}, target={target}, time={time}");
                assert_eq!(
                    new.rule_machine().var("phase").div_euclid(4),
                    old.position_detents()
                );
                a.drives.clear();
                b.drives.clear();
            }
        }
    }
}
#[test]
fn same_rounded_target_does_not_restart_and_reversal_does() {
    let mut old = oracle::RotaryEncoder::new("knob".into(), 0x10, 3, 0x20, 4, 1_000_000);
    let mut new = fixture::rotary("knob", (0x10, 3), (0x20, 4), 1_000_000);
    let mut a = Pads::default();
    let mut b = Pads::default();
    for (time, target) in [
        (0, Some(2.6)),
        (1999, Some(2.51)),
        (2000, None),
        (3000, Some(-1.5)),
        (4999, None),
        (5000, None),
        (7000, None),
        (9000, None),
        (25000, None),
    ] {
        if let Some(v) = target {
            old.set_input("position", v).unwrap();
            new.set_input("position", v).unwrap();
        }
        BusResidentDevice::service(&mut old, &mut a, time);
        new.service(&mut b, time);
        assert_eq!(a.drives, b.drives, "time={time}");
        a.drives.clear();
        b.drives.clear();
    }
}

#[test]
fn descriptor_drives_stm32_and_esp32c3_input_registers() {
    use labwired_core::bus::SystemBus;
    use labwired_core::peripherals::esp32c3::gpio::Esp32c3Gpio;
    use labwired_core::peripherals::gpio::{GpioPort, GpioRegisterLayout};
    use labwired_core::Bus;
    for esp in [false, true] {
        let mut bus = SystemBus::empty();
        let base = if esp { 0x6000_4000 } else { 0x4800_0000 };
        let idr = base + if esp { 0x3c } else { 0x10 };
        let gpio: Box<dyn labwired_core::Peripheral> = if esp {
            Box::new(Esp32c3Gpio::new())
        } else {
            Box::new(GpioPort::new_with_layout(GpioRegisterLayout::Stm32V2))
        };
        bus.add_peripheral("gpio", base, 0x1000, None, gpio);
        bus.gpio_devices.push(Box::new(fixture::rotary(
            "knob",
            (idr, 3),
            (idr, 4),
            1_500_001,
        )));
        bus.gpio_devices[0]
            .as_sim_input()
            .set_input("position", -0.5)
            .unwrap();
        for (now, a, b) in [
            (11, true, true),
            (3010, true, true),
            (3011, true, false),
            (6011, false, false),
            (9011, false, true),
            (12011, true, true),
        ] {
            bus.set_current_cycle(now);
            let _ = bus.tick_peripherals_fully();
            let word = bus.read_u32(idr).unwrap();
            assert_eq!(
                (word & (1 << 3) != 0, word & (1 << 4) != 0),
                (a, b),
                "esp={esp}, now={now}"
            );
        }
    }
}

#[test]
fn multiple_input_changes_before_service_coalesce_to_the_last_target() {
    let mut old = oracle::RotaryEncoder::new("knob".into(), 0x10, 3, 0x20, 4, 1_000_000);
    let mut new = fixture::rotary("knob", (0x10, 3), (0x20, 4), 1_000_000);
    let mut a = Pads::default();
    let mut b = Pads::default();
    for now in [0, 12345, 14344, 14345, 100000] {
        if now == 12345 {
            for v in [3.6, -2.5, 0.0, 1.5, 1.51] {
                old.set_input("position", v).unwrap();
                new.set_input("position", v).unwrap();
            }
        }
        BusResidentDevice::service(&mut old, &mut a, now);
        new.service(&mut b, now);
        assert_eq!(a.drives, b.drives, "now={now}");
        a.drives.clear();
        b.drives.clear();
    }
}
