use labwired_core::bus::bus_trace::BusTrace;
use labwired_core::peripherals::components::GenericI2cDevice;
use labwired_core::peripherals::i2c::{I2c, I2cDevice};
use std::sync::{Arc, Mutex};

fn pca() -> GenericI2cDevice {
    GenericI2cDevice::from_yaml(labwired_config::embedded_device_yaml("pca9685").unwrap(), 0)
        .unwrap()
}
fn write_reg(d: &mut dyn I2cDevice, reg: u8, bytes: &[u8]) {
    d.start();
    d.write(reg);
    for &b in bytes {
        d.write(b);
    }
    d.stop();
}

#[test]
fn detached_slave_retains_its_independent_device_observer() {
    let mut controller = I2c::new();
    controller.attach_traced("i2c1", &BusTrace::default(), Box::new(pca()));
    let pulses = Arc::new(Mutex::new(Vec::new()));
    let out = Arc::clone(&pulses);
    let cell = &controller.attached_devices()[0];
    assert!(cell
        .borrow()
        .subscribe_pwm_output(Arc::new(move |ch, us| if ch == 0 {
            out.lock().unwrap().push(us)
        })));
    {
        let mut d = cell.borrow_mut();
        write_reg(d.as_mut(), 0, &[0x10]);
        write_reg(d.as_mut(), 0xfe, &[121]);
        write_reg(d.as_mut(), 0, &[0x20]);
        write_reg(d.as_mut(), 0x06, &[0, 0, 0x33, 0x01]);
    }
    assert_eq!(pulses.lock().unwrap().len(), 1);
    let mut removed = std::mem::replace(&mut *cell.borrow_mut(), Box::new(pca()));
    write_reg(removed.as_mut(), 0x06, &[0, 0, 0x90, 0x01]);
    assert_eq!(
        pulses.lock().unwrap().len(),
        2,
        "independent device observers stay with the detached device"
    );
}
