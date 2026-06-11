// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

use crate::SimResult;

const REG_CREL: u64 = 0x000;
const REG_ENDN: u64 = 0x004;
const REG_DBTP: u64 = 0x00C;
const REG_TEST: u64 = 0x010;
const REG_CCCR: u64 = 0x018;
const REG_NBTP: u64 = 0x01C;
const REG_TSCC: u64 = 0x020;
const REG_TSCV: u64 = 0x024;
const REG_TOCC: u64 = 0x028;
const REG_TOCV: u64 = 0x02C;
const REG_ECR: u64 = 0x040;
const REG_PSR: u64 = 0x044;
const REG_IR: u64 = 0x080;
const REG_IE: u64 = 0x084;
const REG_ILS: u64 = 0x088;
const REG_ILE: u64 = 0x08C;
const REG_RXGFC: u64 = 0x080 + 0x40;
const REG_RXF0C: u64 = 0x0A0;
const REG_RXF0S: u64 = 0x0A4;
const REG_RXF0A: u64 = 0x0A8;
const REG_RXF1C: u64 = 0x0AC;
const REG_RXF1S: u64 = 0x0B0;
const REG_RXF1A: u64 = 0x0B4;
const REG_TXBC: u64 = 0x0C0;
const REG_TXFQS: u64 = 0x0C4;
const REG_TXBRP: u64 = 0x0CC;
const REG_TXBAR: u64 = 0x0D0;
const REG_TXBCR: u64 = 0x0D4;
const REG_TXBTO: u64 = 0x0D8;
const REG_TXBCF: u64 = 0x0DC;

const CCCR_INIT: u32 = 1 << 0;
const CCCR_CCE: u32 = 1 << 1;
const TXFQS_TFQF: u32 = 1 << 21;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Fdcan {
    crel: u32,
    endn: u32,
    dbtp: u32,
    test: u32,
    cccr: u32,
    nbtp: u32,
    tscc: u32,
    tscv: u32,
    tocc: u32,
    tocv: u32,
    ecr: u32,
    psr: u32,
    ir: u32,
    ie: u32,
    ils: u32,
    ile: u32,
    rxgfc: u32,
    rxf0c: u32,
    rxf0s: u32,
    rxf1c: u32,
    rxf1s: u32,
    txbc: u32,
    txfqs: u32,
    txbrp: u32,
    txbto: u32,
    txbcf: u32,
}

impl Fdcan {
    pub fn new() -> Self {
        Self {
            // Functional M_CAN compatible identity. Exact silicon revision can
            // be pinned later by H563 oracle capture without changing behavior.
            crel: 0x3230_0000,
            endn: 0x8765_4321,
            dbtp: 0,
            test: 0,
            cccr: CCCR_INIT,
            nbtp: 0,
            tscc: 0,
            tscv: 0,
            tocc: 0,
            tocv: 0,
            ecr: 0,
            psr: 0,
            ir: 0,
            ie: 0,
            ils: 0,
            ile: 0,
            rxgfc: 0,
            rxf0c: 0,
            rxf0s: 0,
            rxf1c: 0,
            rxf1s: 0,
            txbc: 0,
            txfqs: TXFQS_TFQF,
            txbrp: 0,
            txbto: 0,
            txbcf: 0,
        }
    }

    #[cfg(test)]
    fn set_interrupts_for_test(&mut self, bits: u32) {
        self.ir |= bits;
    }

    fn config_unlocked(&self) -> bool {
        (self.cccr & (CCCR_INIT | CCCR_CCE)) == (CCCR_INIT | CCCR_CCE)
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            REG_CREL => self.crel,
            REG_ENDN => self.endn,
            REG_DBTP => self.dbtp,
            REG_TEST => self.test,
            REG_CCCR => self.cccr,
            REG_NBTP => self.nbtp,
            REG_TSCC => self.tscc,
            REG_TSCV => self.tscv,
            REG_TOCC => self.tocc,
            REG_TOCV => self.tocv,
            REG_ECR => self.ecr,
            REG_PSR => self.psr,
            REG_IR => self.ir,
            REG_IE => self.ie,
            REG_ILS => self.ils,
            REG_ILE => self.ile,
            REG_RXGFC => self.rxgfc,
            REG_RXF0C => self.rxf0c,
            REG_RXF0S => self.rxf0s,
            REG_RXF1C => self.rxf1c,
            REG_RXF1S => self.rxf1s,
            REG_TXBC => self.txbc,
            REG_TXFQS => self.txfqs,
            REG_TXBRP => self.txbrp,
            REG_TXBTO => self.txbto,
            REG_TXBCF => self.txbcf,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            REG_DBTP if self.config_unlocked() => self.dbtp = value,
            REG_TEST if self.config_unlocked() => self.test = value,
            REG_CCCR => self.write_cccr(value),
            REG_NBTP if self.config_unlocked() => self.nbtp = value,
            REG_TSCC if self.config_unlocked() => self.tscc = value,
            REG_TSCV => self.tscv = value,
            REG_TOCC if self.config_unlocked() => self.tocc = value,
            REG_TOCV => self.tocv = value,
            REG_IR => self.ir &= !value,
            REG_IE => self.ie = value,
            REG_ILS => self.ils = value,
            REG_ILE => self.ile = value & 0x3,
            REG_RXGFC if self.config_unlocked() => self.rxgfc = value,
            REG_RXF0C if self.config_unlocked() => self.rxf0c = value,
            REG_RXF0A => self.ack_rx_fifo0(value),
            REG_RXF1C if self.config_unlocked() => self.rxf1c = value,
            REG_RXF1A => self.ack_rx_fifo1(value),
            REG_TXBC if self.config_unlocked() => self.txbc = value,
            REG_TXBAR => self.request_tx(value),
            REG_TXBCR => {
                self.txbrp &= !value;
                self.txbcf |= value;
            }
            _ => {}
        }
    }

    fn write_cccr(&mut self, value: u32) {
        let allowed = CCCR_INIT
            | CCCR_CCE
            | (1 << 4)
            | (1 << 5)
            | (1 << 6)
            | (1 << 7)
            | (1 << 8)
            | (1 << 9)
            | (1 << 12)
            | (1 << 13);
        self.cccr = value & allowed;
        if (self.cccr & CCCR_INIT) == 0 {
            self.cccr &= !CCCR_CCE;
        }
    }

    fn request_tx(&mut self, value: u32) {
        self.txbrp |= value;
        self.txbrp &= !value;
        self.txbto |= value;
        self.ir |= 1 << 1; // TC
    }

    fn ack_rx_fifo0(&mut self, _value: u32) {
        self.rxf0s = 0;
    }

    fn ack_rx_fifo1(&mut self, _value: u32) {
        self.rxf1s = 0;
    }
}

impl Default for Fdcan {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::Peripheral for Fdcan {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        Ok(((self.read_reg(reg) >> (byte * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        let mask = 0xFF << (byte * 8);
        let current = self.read_reg(reg);
        let next = (current & !mask) | ((value as u32) << (byte * 8));
        self.write_reg(reg, next);
        Ok(())
    }

    // IR is rc_w1: the default byte-decomposed 32-bit write would
    // read-modify-write the still-set bits back into the clear, wiping the
    // whole register (the same failure class the GPIO BSRR / EXTI SWIER
    // silicon diffs pinned on the F103 — atomic word writes are required
    // for w1c registers). Hand the full word straight to write_reg.
    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.read_reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.write_reg(offset & !3, value);
        Ok(())
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    // Go through the trait's word accessors — the bus routes CPU word
    // stores to Peripheral::write_u32, and IR (rc_w1) depends on the
    // write arriving atomically rather than byte-decomposed.
    fn read_u32(dev: &Fdcan, offset: u64) -> u32 {
        Peripheral::read_u32(dev, offset).unwrap()
    }

    fn write_u32(dev: &mut Fdcan, offset: u64, value: u32) {
        Peripheral::write_u32(dev, offset, value).unwrap()
    }

    #[test]
    fn fdcan_core_registers_have_sane_reset_values() {
        let dev = Fdcan::new();

        assert_eq!(read_u32(&dev, 0x000), 0x3230_0000, "CREL");
        assert_eq!(read_u32(&dev, 0x004), 0x8765_4321, "ENDN");
        assert_eq!(read_u32(&dev, 0x018) & 0x3, 0x1, "CCCR starts in init mode");
        assert_eq!(read_u32(&dev, 0x080), 0, "IR");
        assert_eq!(read_u32(&dev, 0x084), 0, "IE");
        assert_eq!(read_u32(&dev, 0x0C4) & (1 << 21), 1 << 21, "TXFQS.TFQF");
    }

    #[test]
    fn fdcan_cccr_protects_timing_writes_until_cce_is_set() {
        let mut dev = Fdcan::new();

        write_u32(&mut dev, 0x01C, 0x1122_3344);
        assert_eq!(read_u32(&dev, 0x01C), 0, "NBTP ignores writes without CCE");

        write_u32(&mut dev, 0x018, 0x3);
        write_u32(&mut dev, 0x01C, 0x1122_3344);
        assert_eq!(
            read_u32(&dev, 0x01C),
            0x1122_3344,
            "NBTP accepts writes with INIT|CCE"
        );

        write_u32(&mut dev, 0x018, 0x0);
        assert_eq!(
            read_u32(&dev, 0x018) & 0x3,
            0,
            "normal mode clears INIT|CCE"
        );
    }

    #[test]
    fn fdcan_interrupt_register_is_write_one_to_clear() {
        let mut dev = Fdcan::new();

        dev.set_interrupts_for_test(0x19);
        assert_eq!(read_u32(&dev, 0x080), 0x19);

        write_u32(&mut dev, 0x080, 0x09);
        assert_eq!(read_u32(&dev, 0x080), 0x10);
    }
}
