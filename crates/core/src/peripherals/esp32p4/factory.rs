use super::gpio::Esp32p4Gpio;
use crate::Peripheral;
use labwired_config::PeripheralConfig;

pub fn try_build(canonical_type: &str, _p_cfg: &PeripheralConfig) -> Option<Box<dyn Peripheral>> {
    match canonical_type {
        "esp32p4_gpio" => Some(Box::new(Esp32p4Gpio::new())),
        _ => None,
    }
}
