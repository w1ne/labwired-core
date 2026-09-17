use super::L4I2c;

// I2C v2 register map (RM0367 §26.7 / RM0351 §39.7).
const CR1: u64 = 0x00;
const CR2: u64 = 0x04;
const TIMINGR: u64 = 0x10;
const ISR: u64 = 0x18;

const PE: u32 = 1 << 0;
const BUSY: u32 = 1 << 15;
const START: u32 = 1 << 13;

/// CR2 arming a 1-byte write to an unattached slave, AUTOEND=0 — the shape
/// the NUCLEO-L073RZ demo issues, and the one the HAL recovers from by
/// toggling PE. Address 0x52 in SADD[7:1].
const ARM_WRITE: u32 = START | (0x52 << 1) | (1 << 16);

fn armed() -> L4I2c {
    let mut i2c = L4I2c::default();
    i2c.write_reg(TIMINGR, 0x0010_0000);
    i2c.write_reg(CR1, PE);
    i2c.write_reg(CR2, ARM_WRITE);
    assert_eq!(
        i2c.read_reg(ISR) & BUSY,
        BUSY,
        "precondition: arming a transfer latches BUSY"
    );
    i2c
}

/// RM0367 §26.7.1 / RM0351 §39.7.1: "When PE=0 ... internal state machines
/// and status bits are put back to their reset value." The model used to
/// store CR1 and leave everything else alone, so BUSY read 1 forever after
/// the HAL's standard NACK recovery (#835).
#[test]
fn clearing_pe_clears_busy() {
    let mut i2c = armed();
    i2c.write_reg(CR1, 0);
    assert_eq!(
        i2c.read_reg(ISR),
        0x0000_0001,
        "PE=0 must return ISR to its reset value (TXE set, BUSY clear)"
    );
}

/// The throughput half of #835: while BUSY is latched, `active()` stays true
/// and the per-cycle engine chain re-arms at +1 forever, which pins the CPU
/// quantum to one instruction for the life of the machine.
#[test]
fn clearing_pe_makes_the_engine_idle() {
    let mut i2c = armed();
    assert!(i2c.active(), "precondition: an armed transfer is active");
    i2c.write_reg(CR1, 0);
    assert!(
        !i2c.active(),
        "a disabled peripheral must not keep the scheduler chain alive"
    );
}

/// Re-enabling must start from a clean engine rather than resuming the
/// transfer that was abandoned.
#[test]
fn re_enabling_starts_clean() {
    let mut i2c = armed();
    i2c.write_reg(CR1, 0);
    i2c.write_reg(CR1, PE);
    assert_eq!(
        i2c.read_reg(ISR) & BUSY,
        0,
        "re-enable must not restore BUSY"
    );
    assert!(!i2c.active());
}

/// A write that leaves PE set must not disturb a transfer in flight —
/// firmware sets interrupt-enable bits in CR1 mid-transfer all the time.
#[test]
fn setting_other_cr1_bits_does_not_reset_the_engine() {
    let mut i2c = armed();
    i2c.write_reg(CR1, PE | (1 << 1) | (1 << 2)); // TXIE | RXIE
    assert_eq!(
        i2c.read_reg(ISR) & BUSY,
        BUSY,
        "an in-flight transfer must survive an unrelated CR1 write"
    );
    assert!(i2c.active());
}

/// Writing CR1 while already disabled is a no-op, not a second reset.
#[test]
fn writing_cr1_while_disabled_is_inert() {
    let mut i2c = L4I2c::default();
    i2c.write_reg(CR1, 0);
    assert_eq!(i2c.read_reg(ISR), 0x0000_0001);
    assert!(!i2c.active());
}
