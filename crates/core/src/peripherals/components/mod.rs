// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

pub mod aht20;
pub mod atecc608a;
pub mod bg770a;
pub mod bme280;
pub mod bmp280;
pub mod bno055;
pub mod button;
pub mod can_testers;
pub mod declarative_analog;
pub mod declarative_display;
pub mod declarative_expr;
pub mod declarative_gpio;
pub mod declarative_i2c;
pub mod declarative_led_strip;
pub mod declarative_logic;
pub mod declarative_regs;
pub mod declarative_spi;
pub mod declarative_uart;
pub mod dht22;
pub mod drv2605;
pub mod h_bridge_motor;
pub mod hc595;
pub mod hc595_7seg;
pub mod i2c_factory;
pub mod ili9341_parallel;
pub mod inmp441;
pub mod iolink_master;
#[cfg(feature = "iolink-native")]
pub mod iolink_native;
pub mod iolink_station;
pub mod keypad;
pub mod lcd1602;
pub mod ldr;
pub mod lipo_charger;
pub mod max30102;
pub mod max7219;
pub mod mcp2515;
pub mod microsd;
pub mod mlx90640;
pub mod mq6;
/// Shared fixture for the per-controller TCA9548A coverage tests. Each I²C
/// controller family exercises the switch from its OWN test module (the
/// register offsets and command opcodes are private there), so the switch
/// topology under test lives here rather than being copied six times.
#[cfg(test)]
pub(crate) mod mux_fixture;
pub mod ntc_thermistor;
pub mod pca9685;
pub mod pn532;
pub mod potentiometer;
pub mod rotary_encoder;
pub mod rule_machine;
pub mod sensirion;
pub mod servo;
pub mod seven_seg_font;
pub mod seven_segment;
pub mod shm_i2c;
pub mod sn74hc165;
pub mod soil_moisture;
pub mod sps30;
pub mod step_dir_motor;
pub mod supply;
pub mod tca9548a;
pub mod tm1637_7seg;
pub mod unipolar_stepper;
/// Hand-written VEML7700 model, retained only as the byte-parity oracle the
/// declarative descriptor is proven identical against (see `veml7700_parity`).
/// The shipping device is `declarative_i2c::VEML7700_KIT`, so this module is
/// test-only.
#[cfg(test)]
pub mod veml7700;
/// Byte-parity harness: the declarative VEML7700 vs the hand-written oracle.
#[cfg(test)]
mod veml7700_parity;
pub mod vl53l1x;
pub mod ydlidar;

pub use aht20::Aht20;
pub use bg770a::QuectelBg770a;
pub use bme280::Bme280;
pub use bmp280::Bmp280;
pub use declarative_display::{
    ili9341, pcd8544, rm67162_gpio_dc, rm67162_hw_dcx, sh1107, ssd1306, ssd1306_128x32,
    ssd1680_tricolor_290, st7789, uc8151d_tricolor_290, DcWiring, DeclarativeDisplayKit,
    GenericDisplay, PlaneView,
};
pub use declarative_gpio::DeclarativeGpioDevice;
pub use declarative_i2c::{DeclarativeI2cKit, GenericI2cDevice};
pub use declarative_led_strip::{
    apa102, ws2812, DeclarativeLedStripKit, GenericLedStrip, LedPixel,
};
pub use declarative_logic::DeclarativeLogicDevice;
pub use declarative_spi::{DeclarativeSpiKit, GenericSpiDevice};
pub use drv2605::{Drv2605, DRV2605_ADDR};
pub use hc595::Hc595;
pub use hc595_7seg::Hc5957Seg;
pub use i2c_factory::{
    build_external_i2c_device, build_i2c_device, build_i2c_tree, i2c_mux_child_ids,
    is_i2c_mux_type, validate_i2c_mux_topology,
};
pub use ili9341_parallel::{Ili9341Parallel, ParallelPins};
pub use iolink_master::{
    IolinkComSpeed, IolinkFrameKind, IolinkLinkState, IolinkMaster, IolinkXfer,
};
pub use lcd1602::Lcd1602;
pub use ldr::Ldr;
pub use max30102::{Max30102, MAX30102_ADDR};
pub use max7219::Max7219;
pub use mlx90640::{Mlx90640, ThermalScene, MLX90640_ADDR};
pub use ntc_thermistor::NtcThermistor;
pub use pca9685::Pca9685;
pub use potentiometer::Potentiometer;
pub use rule_machine::{RuleCtx, RuleMachine};
pub use servo::{LedcServoDriver, McpwmServoDriver, Servo, ServoCal};
pub use shm_i2c::ShmI2c;
pub use sn74hc165::Sn74hc165;
pub use sps30::{Sps30, SPS30_ADDR};
pub use tca9548a::Tca9548a;
pub use tm1637_7seg::Tm1637;
#[cfg(test)]
pub use veml7700::{Veml7700, VEML7700_ADDR};
pub use vl53l1x::Vl53l1x;
