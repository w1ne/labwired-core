// SPDX-License-Identifier: MIT
//! Guest owns TWIM0, sensor routing/acknowledgement and GPIO/GPIOTE reads.
//! Source fixtures live under tests/fixtures; ARM GCC is required.
use labwired_core::{Bus, DebugControl};

#[path = "common/microbit_sensor_guest.rs"]
mod guest;
use guest::{command, observed, RAM};

fn proof(interval: u32) {
    let mut m = guest::machine("motion-drdy.S", interval, |_| {});
    m.run(Some(2_000_000)).unwrap();
    observed(&mut m, 1, 0);
    command(&mut m, 11); // start acceleration XYZ, route off
    m.run(Some(2_000_000)).unwrap();
    observed(&mut m, 1, 0);
    command(&mut m, 9); // expose already-ready acceleration
    observed(&mut m, 0, 1);
    command(&mut m, 12); // acceleration low byte not acknowledgement
    command(&mut m, 13); // X high only: Y/Z still pending
    observed(&mut m, 0, 1);
    command(&mut m, 20);
    assert_eq!(m.bus.read_u8(RAM + 0x18).unwrap(), 0x0e);
    command(&mut m, 13); // duplicate X high cannot clear another axis
    command(&mut m, 14);
    observed(&mut m, 0, 1);
    command(&mut m, 1); // second sensor now drives same collector line
    observed(&mut m, 0, 1); // cannot release acceleration
    command(&mut m, 15); // last accel high; magnetometer still holds line
    observed(&mut m, 0, 1);
    command(&mut m, 4); // mag high-only acknowledgement releases final driver
    observed(&mut m, 1, 1);
    command(&mut m, 2); // keep magnetometer masked for accel controls
    m.run(Some(2_000_000)).unwrap();
    observed(&mut m, 0, 2); // autonomous acceleration recurrence
    command(&mut m, 10); // accel route mask releases, does not ack
    observed(&mut m, 1, 2);
    command(&mut m, 9);
    observed(&mut m, 0, 3);
    command(&mut m, 13);
    command(&mut m, 14);
    command(&mut m, 15);
    observed(&mut m, 1, 3);
    command(&mut m, 19); // cancel new conversion
    command(&mut m, 16); // active-low inactive raw high pulls collector low
    observed(&mut m, 0, 4);
    command(&mut m, 18); // start only X/Y
    m.run(Some(2_000_000)).unwrap();
    observed(&mut m, 1, 4); // active raw low releases board collector
    command(&mut m, 13);
    observed(&mut m, 1, 4);
    command(&mut m, 14); // Z disabled: only X/Y required
    observed(&mut m, 0, 5); // inactive raw high asserts collector
    command(&mut m, 17); // return default polarity
    observed(&mut m, 1, 5);
    command(&mut m, 19);
    m.run(Some(2_000_000)).unwrap();
    observed(&mut m, 1, 5); // no stale completion in power-down
}
#[test]
fn guest_observes_sensor_drdy_profiles() {
    for &interval in guest::profiles() {
        proof(interval);
    }
}
