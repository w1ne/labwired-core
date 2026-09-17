use super::{I2c, I2cDevice, KinetisI2c, KI_C1_MST, KI_C1_TX};
use crate::Peripheral;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

/// The I2C controller's custom `inspect()` emits a `framebuffer` artifact
/// for an attached SSD1306 OLED: metadata always present; the (large) byte
/// payload only when `include_bytes` is requested. This is the pattern that
/// generalizes the bespoke `get_*_framebuffer` accessors.
#[test]
fn inspect_emits_ssd1306_framebuffer_artifact() {
    use crate::inspect::InspectOpts;
    use crate::peripherals::components::Ssd1306;

    let mut i2c = I2c::new();
    i2c.push_slave(Box::new(Ssd1306::new(0x3C)));

    // Summary mode: metadata present, bytes omitted.
    let summary = i2c.inspect(0x4000_5400, "i2c1", &InspectOpts::default());
    assert_eq!(summary.kind, "i2c");
    let fb = summary
        .artifacts
        .iter()
        .find(|a| a.kind == "framebuffer")
        .expect("framebuffer artifact present");
    assert_eq!(fb.id, "i2c@0x3c");
    assert_eq!(fb.meta["w"], 128);
    assert_eq!(fb.meta["h"], 64);
    assert_eq!(fb.meta["format"], "ssd1306_page");
    assert!(
        fb.meta["generation"].is_u64(),
        "cheap change-detection hash"
    );
    assert!(fb.bytes.is_none(), "bytes omitted in summary mode");

    // include_bytes: full GDDRAM payload attached.
    let full = i2c.inspect(
        0x4000_5400,
        "i2c1",
        &InspectOpts {
            include_bytes: true,
            peripheral: None,
        },
    );
    let fb = full
        .artifacts
        .iter()
        .find(|a| a.kind == "framebuffer")
        .expect("framebuffer artifact present");
    assert_eq!(
        fb.bytes.as_ref().map(|b| b.len()),
        Some(128 * 8),
        "1024-byte page-major GDDRAM"
    );
}

struct CountingDevice {
    address: u8,
    reads: Arc<AtomicUsize>,
}

impl CountingDevice {
    fn new(address: u8, reads: Arc<AtomicUsize>) -> Self {
        Self { address, reads }
    }
}

impl I2cDevice for CountingDevice {
    fn address(&self) -> u8 {
        self.address
    }
    fn read(&mut self) -> u8 {
        self.reads.fetch_add(1, Ordering::SeqCst) as u8
    }
    fn write(&mut self, _data: u8) {}
}

#[test]
fn test_i2c_reset_values() {
    let i2c = I2c::new();
    assert_eq!(i2c.read(0x00).unwrap(), 0); // CR1
    assert_eq!(i2c.read(0x04).unwrap(), 0); // CR2
}

#[test]
fn test_i2c_start_bit() {
    let mut i2c = I2c::new();
    // Instant SB: Wire/HAL polls SR1.SB immediately after CR1.START.
    i2c.write(0x01, 0x01).unwrap(); // CR1 START (bit 8) → SR1.SB
    assert_ne!(
        i2c.peek(0x14).unwrap() & 0x01,
        0,
        "SB latches on START write"
    );
}

#[test]
fn test_i2c_full_transfer_flow() {
    use crate::peripherals::components::Mpu6050;
    let mut i2c = I2c::new();
    i2c.push_slave(Box::new(Mpu6050::new(0x50)));

    i2c.write(0x01, 0x01).unwrap(); // START
    for _ in 0..10 {
        i2c.tick();
    }
    assert_ne!(i2c.peek(0x14).unwrap() & 0x01, 0); // SB

    i2c.write(0x10, 0xA0).unwrap(); // addr 0x50<<1 | W
    for _ in 0..20 {
        i2c.tick();
    }
    assert_eq!(i2c.peek(0x14).unwrap() & 0x01, 0); // SB cleared
    assert_ne!(i2c.peek(0x14).unwrap() & 0x02, 0); // ADDR
    assert_ne!(i2c.peek(0x18).unwrap() & 0x01, 0); // MSL
                                                   // TRA (SR2 bit2) must rise on write-address ACK — HAL EV IRQ gates
                                                   // the TXE/BTF path on TRA (RM0008 §26.6.7).
    assert_ne!(
        i2c.peek(0x18).unwrap() & 0x04,
        0,
        "TRA set after write-address ACK"
    );

    i2c.write(0x10, 0x42).unwrap();
    for _ in 0..20 {
        i2c.tick();
    }
    assert_ne!(i2c.peek(0x14).unwrap() & 0x80, 0); // TXE
    assert_ne!(i2c.peek(0x14).unwrap() & 0x04, 0); // BTF

    i2c.write(0x01, 0x02).unwrap(); // STOP (bit 9)
    for _ in 0..10 {
        i2c.tick();
    }
    assert_eq!(
        i2c.peek(0x18).unwrap() & 0x07,
        0,
        "STOP must clear MSL+BUSY+TRA"
    );
}

#[test]
fn f1_write_address_sets_tra_and_level_ev_stays_asserted() {
    use crate::Peripheral;
    struct Ack {
        address: u8,
    }
    impl I2cDevice for Ack {
        fn address(&self) -> u8 {
            self.address
        }
        fn read(&mut self) -> u8 {
            0
        }
        fn write(&mut self, _: u8) {}
    }
    let mut i2c = I2c::new_with_layout(super::I2cRegisterLayout::Stm32F1);
    i2c.push_slave(Box::new(Ack { address: 0x40 }));
    // Enable ITEVTEN|ITBUFEN like HAL_I2C_Master_Transmit_IT.
    i2c.write_u32(0x04, (1 << 9) | (1 << 10)).unwrap();
    i2c.write(0x01, 0x01).unwrap(); // START → SB
    assert!(i2c.tick().irq, "SB with ITEVTEN asserts EV");
    i2c.write(0x10, 0x80).unwrap(); // 0x40 write
                                    // After address ACK: ADDR+TXE+TRA; level EV stays high across ticks.
    assert_ne!(i2c.peek(0x18).unwrap() & 0x04, 0, "TRA");
    assert_ne!(i2c.peek(0x14).unwrap() & 0x80, 0, "TXE");
    assert!(i2c.tick().irq, "level EV while TXE+ITBUFEN");
    assert!(i2c.tick().irq, "level EV re-assert next tick");
    // Clear ADDR via SR1 then SR2 (silicon sequence).
    let _ = i2c.read_u32(0x14).unwrap();
    let _ = i2c.read_u32(0x18).unwrap();
    assert_eq!(i2c.peek(0x14).unwrap() & 0x02, 0, "ADDR cleared by SR1→SR2");
    // TXE still live → EV still asserted for MasterTransmit_TXE.
    assert!(i2c.tick().irq, "TXE keeps EV high after ADDR clear");
}

#[test]
fn test_adxl345_devid_and_axis_read() {
    use crate::peripherals::components::Adxl345;

    let mut i2c = I2c::new();
    let mut sensor = Adxl345::new(0x53);
    sensor.set_sample(256, -128, 64);
    i2c.push_slave(Box::new(sensor));

    i2c.write(0x00, 0x01).unwrap();
    i2c.write(0x01, 0x01).unwrap();
    for _ in 0..10 {
        i2c.tick();
    }
    assert_ne!(i2c.peek(0x14).unwrap() & 0x01, 0);

    i2c.write(0x10, 0xA6).unwrap();
    for _ in 0..20 {
        i2c.tick();
    }
    assert_ne!(i2c.peek(0x14).unwrap() & 0x02, 0);

    i2c.write(0x10, 0x00).unwrap();
    for _ in 0..20 {
        i2c.tick();
    }

    i2c.write(0x01, 0x01).unwrap();
    for _ in 0..10 {
        i2c.tick();
    }
    i2c.write(0x10, 0xA7).unwrap();
    for _ in 0..40 {
        i2c.tick();
    }
    assert_eq!(i2c.read(0x10).unwrap(), 0xE5);

    i2c.write(0x01, 0x02).unwrap();
    for _ in 0..10 {
        i2c.tick();
    }

    i2c.write(0x01, 0x01).unwrap();
    for _ in 0..10 {
        i2c.tick();
    }
    i2c.write(0x10, 0xA6).unwrap();
    for _ in 0..20 {
        i2c.tick();
    }
    i2c.write(0x10, 0x32).unwrap();
    for _ in 0..20 {
        i2c.tick();
    }
    i2c.write(0x01, 0x01).unwrap();
    for _ in 0..10 {
        i2c.tick();
    }
    i2c.write(0x10, 0xA7).unwrap();
    for _ in 0..40 {
        i2c.tick();
    }

    assert_eq!(i2c.read(0x10).unwrap(), 0x00);
    assert_eq!(i2c.read(0x10).unwrap(), 0x01);
    assert_eq!(i2c.read(0x10).unwrap(), 0x80);
    assert_eq!(i2c.read(0x10).unwrap(), 0xFF);
    assert_eq!(i2c.read(0x10).unwrap(), 0x40);
    assert_eq!(i2c.read(0x10).unwrap(), 0x00);
}

#[test]
fn test_i2c_single_byte_read_advances_device_once() {
    let reads = Arc::new(AtomicUsize::new(0));
    let mut i2c = I2c::new();
    i2c.push_slave(Box::new(CountingDevice::new(0x42, reads.clone())));

    i2c.write(0x01, 0x01).unwrap();
    for _ in 0..10 {
        i2c.tick();
    }

    i2c.write(0x10, 0x85).unwrap();
    for _ in 0..40 {
        i2c.tick();
    }

    assert_ne!(i2c.peek(0x14).unwrap() & 0x40, 0);
    assert_eq!(i2c.read(0x10).unwrap(), 0);
    assert_eq!(reads.load(Ordering::SeqCst), 1);
}

// ── STM32L4 (modern) transaction engine ──────────────────────────────────

/// Configure CR2 for a 1-byte 7-bit master write to `addr` with AUTOEND,
/// then load TXDR — the no-device case the tier-1 fixtures exercise.
/// CR2 is a single 32-bit store (matches STM32 HAL).
fn l4_write_xfer(i2c: &mut I2c, addr: u8, byte: u8) {
    use crate::Peripheral;
    i2c.write(0x00, 1).unwrap(); // CR1.PE
    let cr2: u32 = ((addr as u32) << 1) | (1 << 16) | (1 << 25) | (1 << 13);
    i2c.write_u32(0x04, cr2).unwrap();
    i2c.write(0x28, byte).unwrap(); // TXDR: first (only) byte
}

/// Address-only master write (NBYTES=0 + AUTOEND + START) — Wire probe.
fn l4_addr_probe(i2c: &mut I2c, addr: u8) {
    use crate::Peripheral;
    i2c.write(0x00, 1).unwrap(); // CR1.PE
    let cr2: u32 = ((addr as u32) << 1) | (1 << 25) | (1 << 13); // NBYTES=0
    i2c.write_u32(0x04, cr2).unwrap();
}

/// Tick the engine past the address-phase wire-time window so the ACK/NACK
/// verdict lands (TIMINGR left at reset → 144-cycle phase; 256 is safe margin).
/// Run the controller until an armed transfer has fully settled.
///
/// A write transfer costs two phases of wire time — the address phase and
/// the data byte, each `address_phase_cycles()` (floor 64, 144 at the
/// TIMINGR reset value these bare tests use) — so the budget has to clear
/// both with room to spare.
fn l4_settle(i2c: &mut I2c) {
    use crate::Peripheral;
    for _ in 0..1024 {
        i2c.tick();
    }
}

#[test]
fn test_l4_i2c_nack_on_no_device() {
    use super::I2cRegisterLayout;
    let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Stm32L4);

    // Pending window: right after arming START the address phase is still on
    // the wire — START readable, BUSY set, NO NACKF yet (silicon fingerprint).
    l4_write_xfer(&mut i2c, 0x52, 0xAB);
    assert_ne!(
        i2c.peek(0x19).unwrap() & (1 << 7),
        0,
        "BUSY set while pending"
    ); // ISR.BUSY (bit15)
    assert_ne!(
        i2c.read_u32(0x04).unwrap() & (1 << 13),
        0,
        "START still readable"
    );
    assert_eq!(
        i2c.peek(0x18).unwrap() & (1 << 4),
        0,
        "no NACKF while pending"
    );

    // After the wire-time window: NACK on the absent device (AUTOEND clears BUSY).
    l4_settle(&mut i2c);
    assert_ne!(
        i2c.peek(0x18).unwrap() & (1 << 4),
        0,
        "ISR.NACKF when no slave"
    );
    assert_eq!(
        i2c.read_u32(0x04).unwrap() & (1 << 13),
        0,
        "START cleared after phase"
    );
    assert_eq!(i2c.peek(0x19).unwrap() & (1 << 7), 0, "AUTOEND clears BUSY");
    assert_ne!(i2c.peek(0x18).unwrap() & (1 << 5), 0, "AUTOEND sets STOPF");

    // ICR.NACKCF (bit4) + STOPCF (bit5) clear the flags.
    i2c.write(0x1C, (1 << 4) | (1 << 5)).unwrap();
    assert_eq!(
        i2c.peek(0x18).unwrap() & (1 << 4),
        0,
        "NACKF cleared by ICR"
    );
}

#[test]
fn test_l4_i2c_nbytes0_probe_acks_device() {
    use super::I2cRegisterLayout;
    struct AckOnly {
        address: u8,
    }
    impl I2cDevice for AckOnly {
        fn address(&self) -> u8 {
            self.address
        }
        fn read(&mut self) -> u8 {
            0
        }
        fn write(&mut self, _: u8) {}
    }

    let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Stm32L4);
    i2c.push_slave(Box::new(AckOnly { address: 0x40 }));

    l4_addr_probe(&mut i2c, 0x40);
    l4_settle(&mut i2c);
    assert_eq!(
        i2c.peek(0x18).unwrap() & (1 << 4),
        0,
        "no NACKF on present device"
    );
    assert_ne!(
        i2c.peek(0x18).unwrap() & (1 << 6),
        0,
        "TC after address-only"
    );
    assert_ne!(i2c.peek(0x18).unwrap() & (1 << 5), 0, "STOPF via AUTOEND");
    assert_eq!(i2c.peek(0x19).unwrap() & (1 << 7), 0, "BUSY cleared");
}

#[test]
fn test_l4_i2c_ack_delivers_byte_to_device() {
    use super::I2cRegisterLayout;
    use std::sync::atomic::AtomicUsize;
    let writes = Arc::new(AtomicUsize::new(0));

    struct WriteCounter {
        address: u8,
        writes: Arc<AtomicUsize>,
    }
    impl I2cDevice for WriteCounter {
        fn address(&self) -> u8 {
            self.address
        }
        fn read(&mut self) -> u8 {
            0
        }
        fn write(&mut self, _data: u8) {
            self.writes.fetch_add(1, Ordering::SeqCst);
        }
    }

    let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Stm32L4);
    i2c.push_slave(Box::new(WriteCounter {
        address: 0x3C,
        writes: writes.clone(),
    }));

    l4_write_xfer(&mut i2c, 0x3C, 0x42);
    l4_settle(&mut i2c);
    // Attached device ACKs → no NACKF, the byte reaches the device, TC set.
    assert_eq!(
        i2c.peek(0x18).unwrap() & (1 << 4),
        0,
        "no NACKF when device present"
    );
    assert_ne!(
        i2c.peek(0x18).unwrap() & (1 << 6),
        0,
        "TC after byte transferred"
    );
    assert_eq!(writes.load(Ordering::SeqCst), 1);
}

/// Minimal ACK-and-count slave for the master-write ordering tests.
struct WriteSink {
    address: u8,
    writes: Arc<std::sync::atomic::AtomicUsize>,
}
impl I2cDevice for WriteSink {
    fn address(&self) -> u8 {
        self.address
    }
    fn read(&mut self) -> u8 {
        0
    }
    fn write(&mut self, _data: u8) {
        self.writes.fetch_add(1, Ordering::SeqCst);
    }
}

/// PRELOAD ordering (STM32Cube L0/L4/G4/WB `HAL_I2C_Master_Transmit_IT`):
/// firmware writes TXDR BEFORE arming CR2/START. TXDR is a real holding
/// register — the byte must be transmitted once the address phase ACKs, and
/// the 1-byte AUTOEND transfer completes (TC + STOPF) with no tick loop.
#[test]
fn test_l4_i2c_write_preload_before_start() {
    use super::I2cRegisterLayout;
    use std::sync::atomic::AtomicUsize;
    let writes = Arc::new(AtomicUsize::new(0));
    let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Stm32L4);
    i2c.push_slave(Box::new(WriteSink {
        address: 0x40,
        writes: writes.clone(),
    }));

    i2c.write(0x00, 1).unwrap(); // CR1.PE
    i2c.write(0x28, 0x00).unwrap(); // TXDR preloaded FIRST
    let cr2: u32 = (0x40 << 1) | (1 << 16) | (1 << 25) | (1 << 13); // NBYTES=1|AUTOEND|START
    i2c.write_u32(0x04, cr2).unwrap(); // CR2/START after the preload

    // Address phase takes wire time: nothing sent, START still readable.
    assert_eq!(
        writes.load(Ordering::SeqCst),
        0,
        "no byte during address phase"
    );
    assert_ne!(
        i2c.read_u32(0x04).unwrap() & (1 << 13),
        0,
        "START still readable"
    );
    l4_settle(&mut i2c);
    assert_eq!(
        writes.load(Ordering::SeqCst),
        1,
        "preloaded byte reaches slave"
    );
    assert_eq!(
        i2c.peek(0x18).unwrap() & (1 << 4),
        0,
        "no NACKF (slave present)"
    );
    assert_ne!(i2c.peek(0x18).unwrap() & (1 << 6), 0, "TC after transfer");
    assert_ne!(i2c.peek(0x18).unwrap() & (1 << 5), 0, "STOPF via AUTOEND");
    assert_eq!(i2c.peek(0x19).unwrap() & (1 << 7), 0, "BUSY cleared");
}

/// IT ordering (STM32Cube H5 `HAL_I2C_Master_Transmit_IT`): CR2/START first;
/// hardware ACKs the address and asserts ISR.TXIS; only then does the ISR
/// write TXDR. The model must set TXIS on the address ACK (park in
/// DataPending), then complete the byte on the post-TXIS TXDR write.
#[test]
fn test_l4_i2c_write_txis_then_txdr() {
    use super::I2cRegisterLayout;
    use std::sync::atomic::AtomicUsize;
    let writes = Arc::new(AtomicUsize::new(0));
    let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Stm32L4);
    i2c.push_slave(Box::new(WriteSink {
        address: 0x40,
        writes: writes.clone(),
    }));

    i2c.write(0x00, 1).unwrap(); // CR1.PE
    let cr2: u32 = (0x40 << 1) | (1 << 16) | (1 << 25) | (1 << 13); // NBYTES=1|AUTOEND|START
    i2c.write_u32(0x04, cr2).unwrap(); // START first — NO preloaded byte

    // Address phase in flight: no TXIS yet, START still readable.
    assert_eq!(
        i2c.peek(0x18).unwrap() & (1 << 1),
        0,
        "no TXIS during address phase"
    );
    assert_ne!(
        i2c.read_u32(0x04).unwrap() & (1 << 13),
        0,
        "START still readable"
    );

    // After the wire-time window the address ACKed → hardware requests the
    // first byte via TXIS (bit 1), nothing sent yet.
    l4_settle(&mut i2c);
    assert_ne!(
        i2c.peek(0x18).unwrap() & (1 << 1),
        0,
        "TXIS asserted after address ACK"
    );
    assert_eq!(
        writes.load(Ordering::SeqCst),
        0,
        "no byte before TXDR write"
    );

    i2c.write(0x28, 0x00).unwrap(); // ISR writes TXDR after TXIS
    assert_eq!(writes.load(Ordering::SeqCst), 1, "byte sent on TXDR write");
    // Completion is not instant: the byte occupies nine SCL bit-times
    // before TC (and the AUTOEND STOP) land, exactly as on silicon.
    assert_eq!(
        i2c.peek(0x18).unwrap() & (1 << 6),
        0,
        "TC must wait for the byte to clock out"
    );
    l4_settle(&mut i2c);
    assert_ne!(i2c.peek(0x18).unwrap() & (1 << 6), 0, "TC after transfer");
    assert_ne!(i2c.peek(0x18).unwrap() & (1 << 5), 0, "STOPF via AUTOEND");
}

/// Address-NACK must set ISR.NACKF (+STOPF via AUTOEND) in BOTH the preload
/// and IT orderings, so the HAL returns error rather than hanging.
#[test]
fn test_l4_i2c_write_nack_both_orderings() {
    use super::I2cRegisterLayout;

    // Preload ordering: TXDR then START, absent slave.
    let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Stm32L4);
    i2c.write(0x00, 1).unwrap();
    i2c.write(0x28, 0xAB).unwrap();
    let cr2: u32 = (0x52 << 1) | (1 << 16) | (1 << 25) | (1 << 13);
    i2c.write_u32(0x04, cr2).unwrap();
    l4_settle(&mut i2c);
    assert_ne!(
        i2c.peek(0x18).unwrap() & (1 << 4),
        0,
        "NACKF (preload order)"
    );
    assert_ne!(
        i2c.peek(0x18).unwrap() & (1 << 5),
        0,
        "STOPF via AUTOEND (preload order)"
    );
    assert_eq!(
        i2c.peek(0x19).unwrap() & (1 << 7),
        0,
        "BUSY cleared (preload order)"
    );

    // IT ordering: START first, absent slave.
    let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Stm32L4);
    i2c.write(0x00, 1).unwrap();
    let cr2: u32 = (0x52 << 1) | (1 << 16) | (1 << 25) | (1 << 13);
    i2c.write_u32(0x04, cr2).unwrap();
    l4_settle(&mut i2c);
    assert_ne!(i2c.peek(0x18).unwrap() & (1 << 4), 0, "NACKF (IT order)");
    assert_ne!(
        i2c.peek(0x18).unwrap() & (1 << 5),
        0,
        "STOPF via AUTOEND (IT order)"
    );
    assert_eq!(
        i2c.peek(0x19).unwrap() & (1 << 7),
        0,
        "BUSY cleared (IT order)"
    );
}

#[test]
fn i2c_attach_wraps_device_into_shared_log() {
    use crate::bus::bus_trace::{new_log, wrap_i2c, BusPayload};
    use crate::Peripheral;

    let log = new_log();
    let mut i2c = I2c::Kinetis(KinetisI2c::default());

    // device at 0x1E
    struct D;
    impl I2cDevice for D {
        fn address(&self) -> u8 {
            0x1E
        }
        fn read(&mut self) -> u8 {
            0
        }
        fn write(&mut self, _: u8) {}
    }
    // The bus choke point wraps before push; emulate it here.
    i2c.push_slave(wrap_i2c("i2c1", &log, Box::new(D)));

    // Drive START + addr(W) + one data byte through the Kinetis register
    // model via the public `Peripheral::write` MMIO path (the same path
    // every other Kinetis-adjacent test in this module uses to poke
    // registers — `write_reg` itself is private).
    i2c.write(0x02, KI_C1_MST | KI_C1_TX).unwrap(); // START
    i2c.write(0x04, 0x3C).unwrap(); // addr 0x1E + W -> selects device, start()
    i2c.write(0x04, 0xAF).unwrap(); // data -> device.write -> wrapper records

    let snap = log.snapshot();
    assert!(snap
        .iter()
        .any(|e| matches!(&e.payload, BusPayload::I2c { byte, .. } if *byte == 0xAF)));
}

// ── TCA9548A driven through the STM32L4 and Kinetis controllers ─────────
//
// `tests/i2c_mux_tca9548a.rs` proves the switch works through the STM32F1
// legacy peripheral. The L4 and Kinetis engines are separate state machines
// in this file with their own address-resolution sites, and both got the
// `claims_address` / `select_address` change without any switch ever being
// driven through them. These two modules close that.
mod mux_stm32l4 {
    use super::super::{I2c, I2cRegisterLayout};
    use crate::peripherals::components::mux_fixture::{
        bytes_written_to, mux_with_tags, tag_for, MUX_ADDR, SENSOR_ADDR,
    };
    use crate::peripherals::components::tca9548a::Tca9548a;
    use crate::Peripheral;

    /// ICR.NACKCF (bit 4) + STOPCF (bit 5): clear the previous transfer's
    /// verdict so this one's NACKF assertion is about this one.
    fn clear_flags(i2c: &mut I2c) {
        i2c.write(0x1C, (1 << 4) | (1 << 5)).unwrap();
    }

    /// Did the last address phase NACK? ISR.NACKF is bit 4.
    fn nacked(i2c: &I2c) -> bool {
        i2c.peek(0x18).unwrap() & (1 << 4) != 0
    }

    /// One-byte master write (NBYTES=1 + AUTOEND), settled.
    fn write_one(i2c: &mut I2c, addr: u8, byte: u8) {
        clear_flags(i2c);
        super::l4_write_xfer(i2c, addr, byte);
        super::l4_settle(i2c);
    }

    /// One-byte master read (RD_WRN + NBYTES=1 + AUTOEND), settled. The
    /// byte lands in RXDR when the address phase ACKs.
    fn read_one(i2c: &mut I2c, addr: u8) -> u8 {
        clear_flags(i2c);
        i2c.write(0x00, 1).unwrap(); // CR1.PE
        let cr2: u32 = ((addr as u32) << 1)
                | (1 << 10)  // RD_WRN
                | (1 << 16)  // NBYTES = 1
                | (1 << 25)  // AUTOEND
                | (1 << 13); // START
        i2c.write_u32(0x04, cr2).unwrap();
        super::l4_settle(i2c);
        i2c.read(0x24).unwrap() // RXDR
    }

    /// Address-only probe (NBYTES=0): ACK/NACK with no data phase.
    fn probe_acked(i2c: &mut I2c, addr: u8) -> bool {
        clear_flags(i2c);
        super::l4_addr_probe(i2c, addr);
        super::l4_settle(i2c);
        !nacked(i2c)
    }

    fn bus() -> I2c {
        let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Stm32L4);
        let trace = crate::bus::bus_trace::new_log();
        i2c.attach_traced("i2c1", &trace, Box::new(mux_with_tags(4)));
        i2c
    }

    /// Borrow the switch back out of the controller.
    fn with_mux<R>(i2c: &I2c, f: impl FnOnce(&Tca9548a) -> R) -> R {
        let cell = &i2c.attached_devices()[0];
        let traced = cell.borrow();
        let mux = traced
            .as_any()
            .and_then(|a| a.downcast_ref::<Tca9548a>())
            .expect("slave 0 is the switch");
        f(mux)
    }

    /// THE promise: four sensors that cannot be re-addressed, each reached
    /// independently through the L4 engine's own address resolution.
    #[test]
    fn four_sensors_at_one_address_answer_independently() {
        let mut i2c = bus();
        for ch in 0..4u8 {
            write_one(&mut i2c, MUX_ADDR, 1 << ch);
            assert_eq!(
                read_one(&mut i2c, SENSOR_ADDR),
                tag_for(ch),
                "channel {ch} must be answered by the sensor wired to it"
            );
        }
    }

    /// Out-of-order selection: a controller that resolved the address once
    /// and cached it would keep answering with the first channel's sensor.
    #[test]
    fn switching_channels_changes_which_sensor_answers() {
        let mut i2c = bus();
        for ch in [2u8, 0, 3, 1, 3, 0] {
            write_one(&mut i2c, MUX_ADDR, 1 << ch);
            assert_eq!(read_one(&mut i2c, SENSOR_ADDR), tag_for(ch), "channel {ch}");
        }
    }

    #[test]
    fn control_register_reads_back_over_the_bus() {
        let mut i2c = bus();
        write_one(&mut i2c, MUX_ADDR, 0b0000_1010);
        assert!(
            probe_acked(&mut i2c, MUX_ADDR),
            "the switch must ACK its own address"
        );
        // No register pointer on the TCA9548A: a plain read returns the
        // control register.
        assert_eq!(read_one(&mut i2c, MUX_ADDR), 0b0000_1010);
    }

    #[test]
    fn a_sensor_on_a_disabled_channel_does_not_answer() {
        let mut i2c = bus();

        // Reset state: every channel isolated.
        assert!(
            !probe_acked(&mut i2c, SENSOR_ADDR),
            "with all channels disabled the sensor address must NACK, exactly \
                 as an empty bus does"
        );

        // Enable channel 1 only — 0x13 answers, and with channel 1's tag.
        write_one(&mut i2c, MUX_ADDR, 1 << 1);
        assert!(probe_acked(&mut i2c, SENSOR_ADDR));
        assert_eq!(read_one(&mut i2c, SENSOR_ADDR), tag_for(1));

        // Isolate again: it stops answering.
        write_one(&mut i2c, MUX_ADDR, 0x00);
        assert!(
            !probe_acked(&mut i2c, SENSOR_ADDR),
            "re-isolating the switch must take the sensor off the bus again"
        );
    }

    /// A data byte addressed to the sensor must reach the SELECTED
    /// channel's device and no other.
    #[test]
    fn a_write_reaches_only_the_selected_channel() {
        let mut i2c = bus();
        write_one(&mut i2c, MUX_ADDR, 1 << 2);
        write_one(&mut i2c, SENSOR_ADDR, 0x5A);

        with_mux(&i2c, |mux| {
            assert_eq!(bytes_written_to(mux, 2), vec![0x5A]);
            for ch in [0u8, 1, 3] {
                assert!(
                    bytes_written_to(mux, ch).is_empty(),
                    "channel {ch} is isolated and must receive nothing"
                );
            }
        });
    }
}

mod mux_kinetis {
    use super::super::{I2c, I2cRegisterLayout, KI_C1_MST, KI_C1_TX, KI_S_RXAK};
    use crate::peripherals::components::mux_fixture::{
        bytes_written_to, mux_with_tags, tag_for, MUX_ADDR, SENSOR_ADDR,
    };
    use crate::peripherals::components::tca9548a::Tca9548a;
    use crate::Peripheral;

    const REG_C1: u64 = 0x02;
    const REG_S: u64 = 0x03;
    const REG_D: u64 = 0x04;

    /// Did the slave ACK the most recent byte? S.RXAK is set on NAK.
    fn acked(i2c: &I2c) -> bool {
        i2c.peek(REG_S).unwrap() & KI_S_RXAK == 0
    }

    /// START + address(W) + one data byte + STOP, the fsl_i2c byte-at-a-time
    /// master-transmit shape.
    fn write_one(i2c: &mut I2c, addr: u8, byte: u8) {
        i2c.write(REG_C1, KI_C1_MST | KI_C1_TX).unwrap(); // START
        i2c.write(REG_D, addr << 1).unwrap(); // address + W
        i2c.write(REG_D, byte).unwrap();
        i2c.write(REG_C1, KI_C1_TX).unwrap(); // STOP (MST 1→0)
    }

    /// START + address(R), enter master-receive (the HAL's bus-release dummy
    /// read), then clock one real byte out.
    fn read_one(i2c: &mut I2c, addr: u8) -> u8 {
        i2c.write(REG_C1, KI_C1_MST | KI_C1_TX).unwrap(); // START
        i2c.write(REG_D, (addr << 1) | 1).unwrap(); // address + R
        i2c.write(REG_C1, KI_C1_MST).unwrap(); // TX 1→0: enter RX
        let _dummy = i2c.read(REG_D).unwrap(); // HAL bus-release read
        let byte = i2c.read(REG_D).unwrap();
        i2c.write(REG_C1, KI_C1_TX).unwrap(); // STOP
        byte
    }

    /// START + address(W) only: did anything on the bus ACK?
    fn probe_acked(i2c: &mut I2c, addr: u8) -> bool {
        i2c.write(REG_C1, KI_C1_MST | KI_C1_TX).unwrap();
        i2c.write(REG_D, addr << 1).unwrap();
        let ack = acked(i2c);
        i2c.write(REG_C1, KI_C1_TX).unwrap(); // STOP
        ack
    }

    fn bus() -> I2c {
        let mut i2c = I2c::new_with_layout(I2cRegisterLayout::Kinetis);
        let trace = crate::bus::bus_trace::new_log();
        i2c.attach_traced("i2c0", &trace, Box::new(mux_with_tags(4)));
        i2c
    }

    fn with_mux<R>(i2c: &I2c, f: impl FnOnce(&Tca9548a) -> R) -> R {
        let cell = &i2c.attached_devices()[0];
        let traced = cell.borrow();
        let mux = traced
            .as_any()
            .and_then(|a| a.downcast_ref::<Tca9548a>())
            .expect("slave 0 is the switch");
        f(mux)
    }

    #[test]
    fn four_sensors_at_one_address_answer_independently() {
        let mut i2c = bus();
        for ch in 0..4u8 {
            write_one(&mut i2c, MUX_ADDR, 1 << ch);
            assert_eq!(
                read_one(&mut i2c, SENSOR_ADDR),
                tag_for(ch),
                "channel {ch} must be answered by the sensor wired to it"
            );
        }
    }

    #[test]
    fn switching_channels_changes_which_sensor_answers() {
        let mut i2c = bus();
        for ch in [2u8, 0, 3, 1, 3, 0] {
            write_one(&mut i2c, MUX_ADDR, 1 << ch);
            assert_eq!(read_one(&mut i2c, SENSOR_ADDR), tag_for(ch), "channel {ch}");
        }
    }

    #[test]
    fn control_register_reads_back_over_the_bus() {
        let mut i2c = bus();
        write_one(&mut i2c, MUX_ADDR, 0b0000_1010);
        assert!(
            probe_acked(&mut i2c, MUX_ADDR),
            "the switch must ACK its own address"
        );
        assert_eq!(read_one(&mut i2c, MUX_ADDR), 0b0000_1010);
    }

    #[test]
    fn a_sensor_on_a_disabled_channel_does_not_answer() {
        let mut i2c = bus();
        assert!(
            !probe_acked(&mut i2c, SENSOR_ADDR),
            "with all channels disabled the sensor address must NAK (S.RXAK), \
                 exactly as an empty bus does"
        );

        write_one(&mut i2c, MUX_ADDR, 1 << 1);
        assert!(probe_acked(&mut i2c, SENSOR_ADDR));
        assert_eq!(read_one(&mut i2c, SENSOR_ADDR), tag_for(1));

        write_one(&mut i2c, MUX_ADDR, 0x00);
        assert!(
            !probe_acked(&mut i2c, SENSOR_ADDR),
            "re-isolating the switch must take the sensor off the bus again"
        );
    }

    #[test]
    fn a_write_reaches_only_the_selected_channel() {
        let mut i2c = bus();
        write_one(&mut i2c, MUX_ADDR, 1 << 2);
        write_one(&mut i2c, SENSOR_ADDR, 0x5A);

        with_mux(&i2c, |mux| {
            assert_eq!(bytes_written_to(mux, 2), vec![0x5A]);
            for ch in [0u8, 1, 3] {
                assert!(
                    bytes_written_to(mux, ch).is_empty(),
                    "channel {ch} is isolated and must receive nothing"
                );
            }
        });
    }
}
