//! Part-coverage exercise for `lm393-moisture` (resistive LM393 DOUT module).
//!
//! In-crate unit tests under `src/` do not count for scripts/ci/part_coverage.py;
//! this integration test names the type and checks DOUT HIGH=dry / LOW=wet.

use std::borrow::Cow;
use std::collections::HashMap;

use labwired_config::DeviceDescriptor;
use labwired_core::bus::{BusResidentDevice, DevicePins};
use labwired_core::peripherals::components::declarative_gpio::{BoundPin, DeclarativeGpioDevice};
use labwired_core::sim_input::{InputChannel, SimInput};

const TYPE_ID: &str = "lm393-moisture";

#[derive(Default)]
struct Pads {
    idr: HashMap<(u64, u8), bool>,
}

impl DevicePins for Pads {
    fn output_bit(&self, _addr: u64, _bit: u8) -> Option<bool> {
        None
    }
    fn drive_idr_bit(&mut self, addr: u64, bit: u8, high: bool) {
        self.idr.insert((addr, bit), high);
    }
    fn drive_input_bit(&mut self, addr: u64, bit: u8, high: bool) -> bool {
        self.idr.insert((addr, bit), high);
        true
    }
}

fn device() -> DeclarativeGpioDevice {
    let desc = DeviceDescriptor::embedded(TYPE_ID)
        .unwrap()
        .unwrap_or_else(|| panic!("{TYPE_ID} must be embedded"));
    DeclarativeGpioDevice::new(
        "moist".into(),
        &desc,
        Vec::new(),
        vec![BoundPin {
            role: "DOUT".into(),
            addr: 0x2000,
            bit: 0,
        }],
        16_000_000,
        Cow::Owned(vec![InputChannel {
            key: "moisture".into(),
            label: "Moisture".into(),
            unit: "%".into(),
            min: 0.0,
            max: 100.0,
            default: Some(0.0),
        }]),
    )
    .expect("constructs")
}

fn dout_after(moisture: f64) -> bool {
    let mut dev = device();
    SimInput::set_input(&mut dev, "moisture", moisture).unwrap();
    let mut pads = Pads::default();
    BusResidentDevice::service(&mut dev, &mut pads, 1);
    *pads.idr.get(&(0x2000, 0)).expect("DOUT driven")
}

#[test]
fn lm393_moisture_dout_is_high_when_dry_and_low_when_wet() {
    assert!(dout_after(0.0), "moisture 0 → DOUT HIGH");
    assert!(dout_after(49.0), "moisture 49 → DOUT HIGH");
    assert!(!dout_after(50.0), "moisture 50 → DOUT LOW");
    assert!(!dout_after(100.0), "moisture 100 → DOUT LOW");
}
