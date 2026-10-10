use labwired_config::DeviceDescriptor;
use labwired_core::peripherals::components::declarative_gpio::{BoundPin, DeclarativeGpioDevice};

pub const ROWS: usize = 4;
pub const COLS: usize = 4;

pub fn keypad(id: &str, rows: [(u64, u8); ROWS], cols: [(u64, u8); COLS]) -> DeclarativeGpioDevice {
    let descriptor = DeviceDescriptor::embedded("keypad").unwrap().unwrap();
    let bind = |role: &str, pads: [(u64, u8); 4]| {
        pads.into_iter()
            .enumerate()
            .map(|(i, (addr, bit))| BoundPin {
                role: format!("{role}[{i}]"),
                addr,
                bit,
            })
            .collect()
    };
    DeclarativeGpioDevice::new(
        id.into(),
        &descriptor,
        bind("rows", rows),
        bind("cols", cols),
        1_000_000,
        std::borrow::Cow::Owned(vec![labwired_core::sim_input::InputChannel {
            key: "key".into(),
            label: "Key".into(),
            unit: "index".into(),
            min: -1.0,
            max: 15.0,
            default: None,
        }]),
    )
    .unwrap()
}
