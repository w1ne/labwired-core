use labwired_config::DeviceDescriptor;
use labwired_core::peripherals::components::declarative_gpio::{BoundPin, DeclarativeGpioDevice};
pub fn rotary(id: &str, a: (u64, u8), b: (u64, u8), cpu_hz: u64) -> DeclarativeGpioDevice {
    let desc = DeviceDescriptor::embedded("rotary_encoder")
        .unwrap()
        .unwrap();
    DeclarativeGpioDevice::new(
        id.into(),
        &desc,
        vec![],
        vec![
            BoundPin {
                role: "a".into(),
                addr: a.0,
                bit: a.1,
            },
            BoundPin {
                role: "b".into(),
                addr: b.0,
                bit: b.1,
            },
        ],
        cpu_hz,
        std::borrow::Cow::Owned(vec![labwired_core::sim_input::InputChannel {
            key: "position".into(),
            label: "Position".into(),
            unit: "detents".into(),
            min: -1000.0,
            max: 1000.0,
            default: None,
        }]),
    )
    .unwrap()
}

#[allow(dead_code)]
pub struct RotaryHarness {
    device: DeclarativeGpioDevice,
    levels: (bool, bool),
}
#[allow(dead_code)]
impl RotaryHarness {
    pub fn new(id: String, a: u64, ab: u8, b: u64, bb: u8, hz: u64) -> Self {
        Self {
            device: rotary(&id, (a, ab), (b, bb), hz),
            levels: (true, true),
        }
    }
    pub fn service(&mut self, now: u64) -> ((bool, bool), (bool, bool)) {
        use labwired_core::bus::{BusResidentDevice, DevicePins};
        struct Pads;
        impl DevicePins for Pads {
            fn output_bit(&self, _: u64, _: u8) -> Option<bool> {
                None
            }
            fn drive_input_bit(&mut self, _: u64, _: u8, _: bool) -> bool {
                true
            }
            fn drive_idr_bit(&mut self, _: u64, _: u8, _: bool) {}
        }
        self.device.service(&mut Pads, now);
        let m = self.device.rule_machine();
        let levels = (m.pin_level("a").unwrap(), m.pin_level("b").unwrap());
        let changed = (levels.0 != self.levels.0, levels.1 != self.levels.1);
        self.levels = levels;
        (levels, changed)
    }
    pub fn position_detents(&self) -> i64 {
        self.device.rule_machine().var("phase").div_euclid(4)
    }
    pub fn is_moving(&self) -> bool {
        self.device.rule_machine().var("phase") != self.device.rule_machine().var("target")
    }
}
impl labwired_core::sim_input::SimInput for RotaryHarness {
    fn input_channels(&self) -> &[labwired_core::sim_input::InputChannel] {
        labwired_core::sim_input::SimInput::input_channels(&self.device)
    }
    fn set_input(
        &mut self,
        k: &str,
        v: f64,
    ) -> Result<(), labwired_core::sim_input::SimInputError> {
        labwired_core::sim_input::SimInput::set_input(&mut self.device, k, v)
    }
}
