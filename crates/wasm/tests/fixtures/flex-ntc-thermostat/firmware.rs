// SPDX-License-Identifier: MIT
// Portable fan thermostat with hysteresis against the registered NTC
// analog-source model. `setup` runs once, `loop` runs once per control period;
// the fan state lives in firmware memory between calls.
#![no_std]

#[link(wasm_import_module = "labwired")]
extern "C" {
    fn read(port: u32) -> f64;
    fn write(port: u32, value: f64);
    fn advance_ns(delta: u32);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    core::arch::wasm32::unreachable()
}

const FAN_ON_MV: f64 = 2_000.0;
const FAN_OFF_MV: f64 = 1_800.0;

static mut SENSOR_PORT: u32 = 0;
static mut FAN_PORT: u32 = 0;
static mut FAN_ON: bool = false;

#[no_mangle]
pub extern "C" fn setup(sensor_port: u32, fan_port: u32) {
    unsafe {
        SENSOR_PORT = sensor_port;
        FAN_PORT = fan_port;
        FAN_ON = false;
        write(fan_port, 0.0);
    }
}

/// One control period. Return 1 while the fan is commanded on, else 0.
#[export_name = "loop"]
pub extern "C" fn control_period() -> u32 {
    unsafe {
        advance_ns(1_000);
        let millivolts = read(SENSOR_PORT);
        let next = if FAN_ON {
            millivolts > FAN_OFF_MV
        } else {
            millivolts >= FAN_ON_MV
        };
        if next != FAN_ON {
            FAN_ON = next;
            write(FAN_PORT, if next { 1.0 } else { 0.0 });
        }
        u32::from(FAN_ON)
    }
}
