// SPDX-License-Identifier: MIT
//! Guest owns TWIM0, sensor routing/acknowledgement and GPIO/GPIOTE reads.
//! Source fixtures live under tests/fixtures; ARM GCC is required.
use labwired_core::{Bus, DebugControl};

#[path = "common/microbit_sensor_guest.rs"]
mod guest;
use guest::{command, observed, RAM};

fn proof(interval: u32) {
    let mut m = guest::machine("mag-drdy.S", interval, |_| {});
    m.run(Some(2_000_000)).unwrap(); // conversion without sensor/status reads
    assert_eq!(m.bus.read_u8(RAM + 0x1c).unwrap(), 0x40);
    observed(&mut m, 1, 0); // ready but route default masked
    command(&mut m, 1); // expose an already-ready sample
    observed(&mut m, 0, 1);
    command(&mut m, 7); // status is not acknowledgement
    assert_eq!(m.bus.read_u8(RAM + 0x18).unwrap() & 8, 8);
    observed(&mut m, 0, 1);
    command(&mut m, 8); // threshold source is not acknowledgement
    command(&mut m, 3); // low byte is not acknowledgement
    observed(&mut m, 0, 1);
    command(&mut m, 2); // routing mask releases collector
    observed(&mut m, 1, 1);
    command(&mut m, 1); // sample still ready
    observed(&mut m, 0, 2);
    command(&mut m, 4); // high-byte-only acknowledges
    observed(&mut m, 1, 2);
    command(&mut m, 7);
    assert_eq!(m.bus.read_u8(RAM + 0x18).unwrap() & 8, 0);
    m.run(Some(2_000_000)).unwrap(); // second autonomous conversion
    observed(&mut m, 0, 3);
    command(&mut m, 5); // idle preserves unread ready sample
    observed(&mut m, 0, 3);
    command(&mut m, 4);
    m.run(Some(2_000_000)).unwrap();
    observed(&mut m, 1, 3); // no new conversion in idle
    command(&mut m, 6);
    command(&mut m, 5); // cancel in-flight conversion
    m.run(Some(2_000_000)).unwrap();
    observed(&mut m, 1, 3);
}
#[test]
fn guest_observes_mag_drdy_profiles() {
    for &interval in guest::profiles() {
        proof(interval);
    }
}
