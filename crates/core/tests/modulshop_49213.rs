//! Modulshop order #49213 — behavioral checks for new twins.

use labwired_config::DeviceDescriptor;
use labwired_core::bus::{BusResidentDevice, DevicePins};
use labwired_core::peripherals::components::declarative_analog::DeclarativeAnalogKit;
use labwired_core::peripherals::components::declarative_gpio::{BoundPin, DeclarativeGpioDevice};
use labwired_core::peripherals::components::h_bridge_motor::HBridgeMotor;
use labwired_core::peripherals::kit::registry;
use labwired_core::sim_input::SimInput;
use std::collections::HashMap;

#[test]
fn bts7960_alias_resolves_to_l298n_kit() {
    for alias in ["bts7960", "ibt-2", "ibt2"] {
        let kit = registry::lookup(alias).unwrap_or_else(|| panic!("{alias} should resolve"));
        assert_eq!(kit.metadata().device_type.as_ref(), "l298n");
    }
}

#[test]
fn xl4015_clamps_vout_to_vin_minus_dropout() {
    let yaml = labwired_config::embedded_device_yaml("xl4015").expect("embedded");
    let kit = DeclarativeAnalogKit::from_yaml(yaml).expect("kit");
    let mut d = kit.build(0).expect("device");
    // Ask for 12 V out with only 5 V in → clamp to vin - 1.0 = 4.0
    // (stimulus channels reject out-of-range setpoints; the formula clamp is
    // what bites when Vin cannot support the asked Vout).
    d.set_input("vin_v", 5.0).unwrap();
    d.set_input("vout_set_v", 12.0).unwrap();
    // pin_mv = 4.0 * 1000 / 11 ≈ 363
    assert_eq!(d.output_mv(), 363);
    // Ask for 5 V with 12 V in → 5.0 * 1000 / 11 ≈ 454
    d.set_input("vin_v", 12.0).unwrap();
    d.set_input("vout_set_v", 5.0).unwrap();
    assert_eq!(d.output_mv(), 454);
    // Floor of the modelled range: 1.25 V → 113 mV after ÷11
    d.set_input("vout_set_v", 1.25).unwrap();
    assert_eq!(d.output_mv(), 113);
}

#[test]
fn bldc_hall_driver_advances_hall_when_enabled() {
    let yaml = labwired_config::embedded_device_yaml("bldc_hall_driver").expect("embedded");
    let desc = DeviceDescriptor::from_yaml(yaml).expect("parses");
    // Observed: PWM/DIR/ENABLE on addr 1 bits 0/1/2. Driven: HALL on addr 2 bits 0/1/2.
    let mut dev = DeclarativeGpioDevice::new(
        "driver".into(),
        &desc,
        vec![
            BoundPin {
                role: "PWM".into(),
                addr: 1,
                bit: 0,
            },
            BoundPin {
                role: "DIR".into(),
                addr: 1,
                bit: 1,
            },
            BoundPin {
                role: "ENABLE".into(),
                addr: 1,
                bit: 2,
            },
        ],
        vec![
            BoundPin {
                role: "HALL_A".into(),
                addr: 2,
                bit: 0,
            },
            BoundPin {
                role: "HALL_B".into(),
                addr: 2,
                bit: 1,
            },
            BoundPin {
                role: "HALL_C".into(),
                addr: 2,
                bit: 2,
            },
        ],
        1_000_000, // 1 MHz → 1 cycle = 1 µs; commute period 2000 µs
        std::borrow::Cow::Borrowed(&[]),
    )
    .expect("constructs");

    #[derive(Default)]
    struct Pads {
        out: HashMap<u64, u32>,
        idr: HashMap<(u64, u8), bool>,
    }
    impl DevicePins for Pads {
        fn output_bit(&self, addr: u64, bit: u8) -> Option<bool> {
            self.out.get(&addr).map(|w| (w >> bit) & 1 != 0)
        }
        fn drive_idr_bit(&mut self, addr: u64, bit: u8, high: bool) {
            self.idr.insert((addr, bit), high);
        }
        fn drive_input_bit(&mut self, addr: u64, bit: u8, high: bool) -> bool {
            self.idr.insert((addr, bit), high);
            true
        }
    }
    impl Pads {
        fn drive(&mut self, addr: u64, bit: u8, high: bool) {
            let w = self.out.entry(addr).or_insert(0);
            if high {
                *w |= 1 << bit;
            } else {
                *w &= !(1 << bit);
            }
        }
        fn hall(&self) -> (bool, bool, bool) {
            (
                *self.idr.get(&(2, 0)).unwrap_or(&false),
                *self.idr.get(&(2, 1)).unwrap_or(&false),
                *self.idr.get(&(2, 2)).unwrap_or(&false),
            )
        }
    }

    let mut pads = Pads::default();
    // Settle at defaults (ENABLE low) — Hall should stay at rest 101.
    BusResidentDevice::service(&mut dev, &mut pads, 0);
    let rest = pads.hall();
    // Enable and walk several commute periods.
    pads.drive(1, 2, true); // ENABLE high
    BusResidentDevice::service(&mut dev, &mut pads, 1);
    let mut saw_change = false;
    let mut prev = pads.hall();
    for i in 1..=10 {
        let now = 1 + i * 2000;
        BusResidentDevice::service(&mut dev, &mut pads, now);
        let now_h = pads.hall();
        if now_h != prev {
            saw_change = true;
        }
        prev = now_h;
    }
    assert!(
        saw_change,
        "Hall pattern must advance while ENABLE is high; rest={rest:?} last={prev:?}"
    );
}

#[test]
fn ibt2_effort_matches_modulshop_arduino_example() {
    // speed>0 → LPWM, speed<0 → RPWM, 0 → coast; both EN high.
    let m = HBridgeMotor::new_ibt2("ibt", 10, 11, Some(12), Some(13));
    m.on_gpio_edge(12, true, 0);
    m.on_gpio_edge(13, true, 1);
    m.on_gpio_edge(10, true, 2); // LPWM
    m.on_gpio_edge(11, false, 3);
    assert_eq!(m.effort(), 1.0);
    m.on_gpio_edge(10, false, 4);
    m.on_gpio_edge(11, true, 5);
    assert_eq!(m.effort(), -1.0);
}
