use super::*;
use crate::Peripheral;

/// A minimal register-file slave: the shape almost every I²C sensor has.
/// Write one byte to select a register, then read to stream from it.
#[derive(Debug)]
struct FakeSensor {
    addr: u8,
    regs: [u8; 4],
    pointer: usize,
    starts: usize,
    stops: usize,
}

impl FakeSensor {
    fn new(addr: u8) -> Self {
        Self {
            addr,
            regs: [0xA1, 0xB2, 0xC3, 0xD4],
            pointer: 0,
            starts: 0,
            stops: 0,
        }
    }
}

impl I2cDevice for FakeSensor {
    fn address(&self) -> u8 {
        self.addr
    }
    fn read(&mut self) -> u8 {
        let b = self.regs[self.pointer % self.regs.len()];
        self.pointer += 1;
        b
    }
    fn write(&mut self, data: u8) {
        self.pointer = data as usize;
    }
    fn start(&mut self) {
        self.starts += 1;
    }
    fn stop(&mut self) {
        self.stops += 1;
    }
}

fn enabled() -> I2c {
    let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Efr32s2);
    i2c.write_u32(EFR_I2C_EN, EFR_EN_EN).unwrap();
    i2c
}

fn inner(i2c: &I2c) -> &Efr32s2I2c {
    match i2c {
        I2c::Efr32s2(i) => i,
        _ => panic!("wrong layout"),
    }
}

fn clear_flags(i2c: &mut I2c) {
    i2c.write_u32(EFR_I2C_IF, 0xFFFF_FFFF).unwrap();
}

fn flags(i2c: &I2c) -> u32 {
    i2c.read_u32(EFR_I2C_IF).unwrap()
}

/// `(addr << 1) | rw`, the byte firmware writes to TXDATA after a START.
fn addr_byte(addr: u8, reading: bool) -> u32 {
    ((addr as u32) << 1) | u32::from(reading)
}

#[test]
fn the_layout_resolves_by_name_and_reports_itself() {
    let i2c = I2c::new_with_layout(I2cRegisterLayout::Efr32s2);
    assert_eq!(i2c.register_layout(), I2cRegisterLayout::Efr32s2);
    assert_eq!(
        "efr32s2".parse::<I2cRegisterLayout>().unwrap(),
        I2cRegisterLayout::Efr32s2
    );
}

#[test]
fn ipversion_reads_the_header_reset_value() {
    let i2c = I2c::new_with_layout(I2cRegisterLayout::Efr32s2);
    // `_I2C_IPVERSION_RESETVALUE` = 0, and BRD2709A reads 0 over SWD.
    assert_eq!(i2c.read_u32(EFR_I2C_IPVERSION).unwrap(), 0);
}

/// The whole `Wire.beginTransmission / write / endTransmission` path.
#[test]
fn a_write_transaction_reaches_the_slave() {
    let mut i2c = enabled();
    i2c.push_slave(Box::new(FakeSensor::new(0x48)));
    clear_flags(&mut i2c);

    i2c.write_u32(EFR_I2C_CMD, EFR_CMD_START).unwrap();
    assert_eq!(flags(&i2c) & EFR_IF_START, EFR_IF_START);

    i2c.write_u32(EFR_I2C_TXDATA, addr_byte(0x48, false))
        .unwrap();
    assert_eq!(flags(&i2c) & EFR_IF_ACK, EFR_IF_ACK, "the slave answered");
    assert_eq!(flags(&i2c) & EFR_IF_NACK, 0);

    i2c.write_u32(EFR_I2C_TXDATA, 2).unwrap(); // select register 2
    i2c.write_u32(EFR_I2C_CMD, EFR_CMD_STOP).unwrap();
    assert_eq!(flags(&i2c) & EFR_IF_MSTOP, EFR_IF_MSTOP);

    let dev = &inner(&i2c).attached_devices[0];
    assert_eq!(dev.borrow().address(), 0x48);
}

/// An address nobody claims must NACK. This is the difference between a
/// sketch finding out its sensor is not wired and one that appears to work.
#[test]
fn an_unclaimed_address_nacks() {
    let mut i2c = enabled();
    i2c.push_slave(Box::new(FakeSensor::new(0x48)));
    clear_flags(&mut i2c);

    i2c.write_u32(EFR_I2C_CMD, EFR_CMD_START).unwrap();
    i2c.write_u32(EFR_I2C_TXDATA, addr_byte(0x77, false))
        .unwrap();

    assert_eq!(flags(&i2c) & EFR_IF_NACK, EFR_IF_NACK);
    assert_eq!(flags(&i2c) & EFR_IF_ACK, 0);
    assert_eq!(
        i2c.read_u32(EFR_I2C_STATE).unwrap() & EFR_STATE_NACKED,
        EFR_STATE_NACKED
    );
}

/// `Wire.requestFrom`: START, address with R, then a byte per ACK.
#[test]
fn a_read_transaction_streams_bytes_from_the_slave() {
    let mut i2c = enabled();
    i2c.push_slave(Box::new(FakeSensor::new(0x48)));
    clear_flags(&mut i2c);

    i2c.write_u32(EFR_I2C_CMD, EFR_CMD_START).unwrap();
    i2c.write_u32(EFR_I2C_TXDATA, addr_byte(0x48, true))
        .unwrap();

    assert_eq!(
        i2c.read_u32(EFR_I2C_STATUS).unwrap() & EFR_STATUS_RXDATAV,
        EFR_STATUS_RXDATAV,
        "the first byte is ready once the address is acked"
    );
    assert_eq!(i2c.read_u32(EFR_I2C_RXDATA).unwrap(), 0xA1);

    // ACK asks for another byte.
    i2c.write_u32(EFR_I2C_CMD, EFR_CMD_ACK).unwrap();
    assert_eq!(i2c.read_u32(EFR_I2C_RXDATA).unwrap(), 0xB2);
    i2c.write_u32(EFR_I2C_CMD, EFR_CMD_ACK).unwrap();
    assert_eq!(i2c.read_u32(EFR_I2C_RXDATA).unwrap(), 0xC3);

    // NACK ends it: no further byte is fetched.
    i2c.write_u32(EFR_I2C_CMD, EFR_CMD_NACK | EFR_CMD_STOP)
        .unwrap();
    assert_eq!(
        i2c.read_u32(EFR_I2C_STATUS).unwrap() & EFR_STATUS_RXDATAV,
        0
    );
}

/// Reading RXDATA CONSUMES; reading RXDATAP does not. A driver that peeks
/// must not lose a byte.
#[test]
fn rxdatap_peeks_where_rxdata_consumes() {
    let mut i2c = enabled();
    i2c.push_slave(Box::new(FakeSensor::new(0x48)));
    i2c.write_u32(EFR_I2C_CMD, EFR_CMD_START).unwrap();
    i2c.write_u32(EFR_I2C_TXDATA, addr_byte(0x48, true))
        .unwrap();

    assert_eq!(i2c.read_u32(EFR_I2C_RXDATAP).unwrap(), 0xA1);
    assert_eq!(i2c.read_u32(EFR_I2C_RXDATAP).unwrap(), 0xA1, "still there");
    assert_eq!(i2c.read_u32(EFR_I2C_RXDATA).unwrap(), 0xA1, "now taken");
    assert_eq!(
        i2c.read_u32(EFR_I2C_STATUS).unwrap() & EFR_STATUS_RXDATAV,
        0
    );
}

/// ⚠️ `peek` is a side-effect-free probe for observers. It must not consume
/// the RX byte — the IADC hit exactly this and read back zeroes.
#[test]
fn peeking_the_data_register_does_not_consume_the_byte() {
    let mut i2c = enabled();
    i2c.push_slave(Box::new(FakeSensor::new(0x48)));
    i2c.write_u32(EFR_I2C_CMD, EFR_CMD_START).unwrap();
    i2c.write_u32(EFR_I2C_TXDATA, addr_byte(0x48, true))
        .unwrap();

    assert_eq!(i2c.peek(EFR_I2C_RXDATA), Some(0xA1));
    assert_eq!(i2c.peek(EFR_I2C_RXDATA), Some(0xA1));
    assert_eq!(i2c.read_u32(EFR_I2C_RXDATA).unwrap(), 0xA1);
}

/// The register-then-read idiom: write a pointer, repeated START, read.
#[test]
fn a_repeated_start_switches_direction_without_releasing_the_bus() {
    let mut i2c = enabled();
    i2c.push_slave(Box::new(FakeSensor::new(0x48)));
    clear_flags(&mut i2c);

    i2c.write_u32(EFR_I2C_CMD, EFR_CMD_START).unwrap();
    i2c.write_u32(EFR_I2C_TXDATA, addr_byte(0x48, false))
        .unwrap();
    i2c.write_u32(EFR_I2C_TXDATA, 3).unwrap(); // pointer := 3
    clear_flags(&mut i2c);

    i2c.write_u32(EFR_I2C_CMD, EFR_CMD_START).unwrap();
    assert_eq!(
        flags(&i2c) & EFR_IF_RSTART,
        EFR_IF_RSTART,
        "a START while the bus is held is a REPEATED start"
    );
    assert_eq!(flags(&i2c) & EFR_IF_START, 0);

    i2c.write_u32(EFR_I2C_TXDATA, addr_byte(0x48, true))
        .unwrap();
    assert_eq!(
        i2c.read_u32(EFR_I2C_RXDATA).unwrap(),
        0xD4,
        "reads from the register the pointer selected"
    );
}

#[test]
fn a_disabled_controller_does_nothing_at_all() {
    let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Efr32s2);
    i2c.push_slave(Box::new(FakeSensor::new(0x48)));
    clear_flags(&mut i2c);

    i2c.write_u32(EFR_I2C_CMD, EFR_CMD_START).unwrap();
    i2c.write_u32(EFR_I2C_TXDATA, addr_byte(0x48, false))
        .unwrap();
    assert_eq!(flags(&i2c), 0, "no START, no ACK, no NACK");
    // BUSY is still the power-on 1 here: a DISABLED controller cannot have
    // driven the bus, so it has not learned the bus is idle either. What
    // this case is about is that nothing else moved — see `flags` above and
    // MASTER below, which a real START would have set.
    let state = i2c.read_u32(EFR_I2C_STATE).unwrap();
    assert_eq!(
        state & EFR_STATE_MASTER,
        0,
        "a disabled controller never masters"
    );
}

#[test]
fn state_reports_busy_and_master_between_start_and_stop() {
    let mut i2c = enabled();
    i2c.push_slave(Box::new(FakeSensor::new(0x48)));
    // ⚠️ BUSY is SET on a controller nothing has driven yet — measured on a
    // BRD2709A over SWD, `_I2C_STATE_RESETVALUE` 0x00000001. It clears once
    // the controller learns where the bus is, which is what emlib's opening
    // ABORT is for.
    assert_eq!(
        i2c.read_u32(EFR_I2C_STATE).unwrap() & EFR_STATE_BUSY,
        EFR_STATE_BUSY,
        "power-on BUSY, before any ABORT"
    );
    i2c.write_u32(EFR_I2C_CMD, EFR_CMD_ABORT).unwrap();
    assert_eq!(i2c.read_u32(EFR_I2C_STATE).unwrap() & EFR_STATE_BUSY, 0);

    i2c.write_u32(EFR_I2C_CMD, EFR_CMD_START).unwrap();
    let state = i2c.read_u32(EFR_I2C_STATE).unwrap();
    assert_eq!(state & EFR_STATE_BUSY, EFR_STATE_BUSY);
    assert_eq!(state & EFR_STATE_MASTER, EFR_STATE_MASTER);

    i2c.write_u32(EFR_I2C_CMD, EFR_CMD_STOP).unwrap();
    assert_eq!(i2c.read_u32(EFR_I2C_STATE).unwrap() & EFR_STATE_BUSY, 0);
}

/// A slave must see the framing, not just the bytes: a sensor that latches
/// on STOP (most of them) never commits if the controller does not deliver
/// one.
#[test]
fn start_and_stop_reach_the_slave() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[derive(Debug)]
    struct Counting {
        starts: Arc<AtomicUsize>,
        stops: Arc<AtomicUsize>,
    }
    impl I2cDevice for Counting {
        fn address(&self) -> u8 {
            0x48
        }
        fn read(&mut self) -> u8 {
            0
        }
        fn write(&mut self, _data: u8) {}
        fn start(&mut self) {
            self.starts.fetch_add(1, Ordering::Relaxed);
        }
        fn stop(&mut self) {
            self.stops.fetch_add(1, Ordering::Relaxed);
        }
    }

    let starts = Arc::new(AtomicUsize::new(0));
    let stops = Arc::new(AtomicUsize::new(0));
    let mut i2c = enabled();
    i2c.push_slave(Box::new(Counting {
        starts: starts.clone(),
        stops: stops.clone(),
    }));

    i2c.write_u32(EFR_I2C_CMD, EFR_CMD_START).unwrap();
    i2c.write_u32(EFR_I2C_TXDATA, addr_byte(0x48, false))
        .unwrap();
    i2c.write_u32(EFR_I2C_CMD, EFR_CMD_STOP).unwrap();

    assert_eq!(starts.load(Ordering::Relaxed), 1);
    assert_eq!(stops.load(Ordering::Relaxed), 1);
}

#[test]
fn the_flag_register_is_write_one_to_clear_and_ien_gates_the_irq() {
    let mut i2c = enabled();
    i2c.push_slave(Box::new(FakeSensor::new(0x48)));
    clear_flags(&mut i2c);

    i2c.write_u32(EFR_I2C_CMD, EFR_CMD_START).unwrap();
    assert_eq!(flags(&i2c) & EFR_IF_START, EFR_IF_START);
    assert!(!i2c.tick().irq, "IEN clear: no interrupt");

    i2c.write_u32(EFR_I2C_IEN, EFR_IF_START).unwrap();
    assert!(i2c.tick().irq);

    i2c.write_u32(EFR_I2C_IF, EFR_IF_START).unwrap();
    assert_eq!(flags(&i2c) & EFR_IF_START, 0);
    assert!(!i2c.tick().irq);
}

/// A word write of CMD must apply the whole word at once. Byte-slicing it
/// would apply START and STOP as two separate commands.
#[test]
fn a_word_write_of_cmd_is_one_command_not_four_bytes() {
    let mut i2c = enabled();
    i2c.push_slave(Box::new(FakeSensor::new(0x48)));
    i2c.write_u32(EFR_I2C_CMD, EFR_CMD_START).unwrap();
    i2c.write_u32(EFR_I2C_TXDATA, addr_byte(0x48, true))
        .unwrap();
    clear_flags(&mut i2c);

    i2c.write_u32(EFR_I2C_CMD, EFR_CMD_NACK | EFR_CMD_STOP)
        .unwrap();
    assert_eq!(flags(&i2c) & EFR_IF_MSTOP, EFR_IF_MSTOP);
    assert_eq!(i2c.read_u32(EFR_I2C_STATE).unwrap() & EFR_STATE_BUSY, 0);
}
