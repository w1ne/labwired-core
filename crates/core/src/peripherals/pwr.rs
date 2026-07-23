// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! PWR (power-control) peripheral — STM32L4 layout.
//!
//! Reset values verified against real NUCLEO-L476RG silicon via SWD
//! register dump. Used by every HAL-generated firmware: HAL_Init() calls
//! HAL_PWREx_ControlVoltageScaling() before any RCC PLL reconfiguration,
//! and a missing PWR peripheral bus-faults at the very first store.

use crate::SimResult;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Pwr {
    cr1: u32,
    cr2: u32,
    cr3: u32,
    cr4: u32,
    sr1: u32,
    sr2: u32,
    scr: u32,
    pucra: u32,
    pdcra: u32,
    pucrb: u32,
    pdcrb: u32,
    pucrc: u32,
    pdcrc: u32,
    pucrd: u32,
    pdcrd: u32,
    pucre: u32,
    pdcre: u32,
    pucrf: u32,
    pdcrf: u32,
    pucrg: u32,
    pdcrg: u32,
    pucrh: u32,
    pdcrh: u32,
    pucri: u32,
    pdcri: u32,
}

impl Pwr {
    pub fn new() -> Self {
        // Hardware-verified reset state from NUCLEO-L476RG SWD dump:
        //   CR1 = 0x0000_0200  VOS = 01 (range 1, default).
        //   CR3 = 0x0000_8000  EIWUL = 1 (internal wake-up line enabled).
        //   SR2 = 0x0000_0100  REGLPF = 1 (low-power regulator stabilised).
        // Other registers reset to 0.
        Self {
            cr1: 0x0000_0200,
            cr2: 0,
            cr3: 0x0000_8000,
            cr4: 0,
            sr1: 0,
            sr2: 0x0000_0100,
            scr: 0,
            pucra: 0,
            pdcra: 0,
            pucrb: 0,
            pdcrb: 0,
            pucrc: 0,
            pdcrc: 0,
            pucrd: 0,
            pdcrd: 0,
            pucre: 0,
            pdcre: 0,
            pucrf: 0,
            pdcrf: 0,
            pucrg: 0,
            pdcrg: 0,
            pucrh: 0,
            pdcrh: 0,
            pucri: 0,
            pdcri: 0,
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr1,
            0x04 => self.cr2,
            0x08 => self.cr3,
            0x0C => self.cr4,
            0x10 => self.sr1,
            0x14 => self.sr2,
            0x18 => self.scr,
            0x20 => self.pucra,
            0x24 => self.pdcra,
            0x28 => self.pucrb,
            0x2C => self.pdcrb,
            0x30 => self.pucrc,
            0x34 => self.pdcrc,
            0x38 => self.pucrd,
            0x3C => self.pdcrd,
            0x40 => self.pucre,
            0x44 => self.pdcre,
            0x48 => self.pucrf,
            0x4C => self.pdcrf,
            0x50 => self.pucrg,
            0x54 => self.pdcrg,
            0x58 => self.pucrh,
            0x5C => self.pdcrh,
            0x60 => self.pucri,
            0x64 => self.pdcri,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            // CR1 writable bits: LPMS[2:0], DBP, LPR, VOS[1:0], R1MODE, RRSTP — keep 17:0.
            0x00 => self.cr1 = value & 0x0003_FFFF,
            0x04 => self.cr2 = value,
            0x08 => self.cr3 = value,
            0x0C => self.cr4 = value,
            // SR1 / SR2 are read-mostly. Some bits are W1C via SCR; for
            // simplicity allow direct writes to leave register state.
            0x10 => self.sr1 = value,
            0x14 => self.sr2 = value,
            // SCR is write-1-to-clear into SR1 wake-up flags.
            0x18 => {
                // bits [4:0] clear corresponding SR1 wake-up flags; bit 8
                // clears SBF (standby flag); bit 9 clears WUFI.
                self.sr1 &= !(value & 0x0000_031F);
                self.scr = 0;
            }
            0x20 => self.pucra = value,
            0x24 => self.pdcra = value,
            0x28 => self.pucrb = value,
            0x2C => self.pdcrb = value,
            0x30 => self.pucrc = value,
            0x34 => self.pdcrc = value,
            0x38 => self.pucrd = value,
            0x3C => self.pdcrd = value,
            0x40 => self.pucre = value,
            0x44 => self.pdcre = value,
            0x48 => self.pucrf = value,
            0x4C => self.pdcrf = value,
            0x50 => self.pucrg = value,
            0x54 => self.pdcrg = value,
            0x58 => self.pucrh = value,
            0x5C => self.pdcrh = value,
            0x60 => self.pucri = value,
            0x64 => self.pdcri = value,
            _ => {}
        }
    }
}

impl Default for Pwr {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::Peripheral for Pwr {
    // Inert walk: register bank (voltage scaling resolves in the write path); tick() is the trait-default no-op.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        Ok(((self.read_reg(reg) >> (byte * 8)) & 0xFF) as u8)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        let mut v = self.read_reg(reg);
        let mask: u32 = 0xFF << (byte * 8);
        v = (v & !mask) | ((value as u32) << (byte * 8));
        self.write_reg(reg, v);
        Ok(())
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

/// PWR — STM32H5 layout (RM0481 §10).
///
/// Register surface and reset values pinned to a NUCLEO-H563ZI SWD probe
/// (silicon capture 2026-06-11): PMCR=0x0C, VOSSR=0x2008
/// (ACTVOSRDY|VOSRDY, ACTVOS=Scale3), SCCR=0x100, VMSR bit 20 set, all
/// other registers 0. Voltage scaling completes instantly in the sim:
/// `VOSSR = (VOSCR.VOS << 14) | ACTVOSRDY | VOSRDY` tracks every VOSCR
/// write — silicon-probed across Scale3 -> Scale0 -> Scale2 -> Scale3
/// (VOSSR read 0x2008 / 0xE008 / 0x6008 / 0x2008). Foreign H5 firmware
/// (HAL and embassy alike) polls VOSSR.VOSRDY before touching the PLL.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct PwrH5 {
    pmcr: u32,
    voscr: u32,
    bdcr: u32,
    dbpcr: u32,
    ucpdr: u32,
    sccr: u32,
    vmcr: u32,
    usbscr: u32,
    wucr: u32,
    ioretr: u32,
    seccfgr: u32,
    privcfgr: u32,
}

impl PwrH5 {
    pub fn new() -> Self {
        Self {
            pmcr: 0x0000_000C,
            sccr: 0x0000_0100,
            ..Default::default()
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.pmcr,
            0x04 => 0, // PMSR
            0x10 => self.voscr,
            // VOSSR: ACTVOS mirrors the requested VOS, both ready bits up.
            0x14 => (((self.voscr >> 4) & 0x3) << 14) | (1 << 13) | (1 << 3),
            0x20 => self.bdcr,
            0x24 => self.dbpcr,
            0x28 => 0, // BDSR
            0x2C => self.ucpdr,
            0x30 => self.sccr,
            0x34 => self.vmcr,
            0x38 => self.usbscr,
            0x3C => 0x0010_0000, // VMSR (silicon-pinned static status)
            0x44 => 0,           // WUSR
            0x48 => self.wucr,
            0x50 => self.ioretr,
            0x100 => self.seccfgr,
            0x104 => self.privcfgr,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.pmcr = value,
            0x10 => self.voscr = value & 0x30, // VOS[5:4]
            0x20 => self.bdcr = value,
            0x24 => self.dbpcr = value & 0x1,
            0x2C => self.ucpdr = value,
            0x30 => self.sccr = value,
            0x34 => self.vmcr = value,
            0x38 => self.usbscr = value,
            0x40 => {} // WUSCR w1c into WUSR (always 0 here)
            0x48 => self.wucr = value,
            0x50 => self.ioretr = value,
            0x100 => self.seccfgr = value,
            0x104 => self.privcfgr = value,
            _ => {}
        }
    }
}

impl crate::Peripheral for PwrH5 {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        Ok(((self.read_reg(reg) >> (byte * 8)) & 0xFF) as u8)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        let mut v = self.read_reg(reg);
        let mask: u32 = 0xFF << (byte * 8);
        v = (v & !mask) | ((value as u32) << (byte * 8));
        self.write_reg(reg, v);
        Ok(())
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

/// PWR — STM32H7 layout (RM0468 / RM0433).
///
/// Voltage scaling completes instantly in the sim so foreign H7 firmware
/// (stm32h7xx-hal / embassy) clears its bring-up polls: `pwr.freeze()` waits on
/// D3CR/SRDCR (0x18) VOSRDY (bit 13) and CSR1 (0x04) ACTVOSRDY (bit 13), and
/// requires CSR1.ACTVOS[15:14] to mirror the D3CR.VOS[15:14] it just wrote.
/// CR1/CR2/CR3 round-trip (the HAL sets CR3.LDOEN/SMPSEN then asserts the
/// read-back, sets CR1.DBP then polls it, and enables CR2.BREN then polls
/// BRRDY). NOT silicon-verified — reference-manual-derived (no H7 bench part).
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct PwrH7 {
    cr1: u32,
    cr2: u32,
    cr3: u32,
    /// D3CR/SRDCR (0x18) — only VOS[15:14] is stored; VOSRDY is synthesized.
    d3cr: u32,
}

impl PwrH7 {
    pub fn new() -> Self {
        Self {
            // CR3 reset 0x00000006: LDOEN (bit 1) + SDEN/SMPSEN (bit 2) both up
            // out of reset, matching silicon — the HAL's default LDO supply path
            // asserts CR3.LDOEN is set.
            cr3: 0x0000_0006,
            ..Default::default()
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr1,
            // CSR1: ACTVOS[15:14] mirrors the requested D3CR.VOS, ACTVOSRDY (13) up.
            0x04 => (self.d3cr & (0x3 << 14)) | (1 << 13),
            // CR2: BRRDY (bit 16) follows BREN (bit 0) — backup regulator ready.
            0x08 => {
                if self.cr2 & 1 != 0 {
                    self.cr2 | (1 << 16)
                } else {
                    self.cr2 & !(1 << 16)
                }
            }
            0x0C => self.cr3,
            // D3CR/SRDCR: stored VOS[15:14] with VOSRDY (bit 13) always ready.
            0x18 => self.d3cr | (1 << 13),
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.cr1 = value,
            0x08 => self.cr2 = value,
            0x0C => self.cr3 = value,
            0x18 => self.d3cr = value & (0x3 << 14),
            _ => {}
        }
    }
}

impl crate::Peripheral for PwrH7 {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        Ok(((self.read_reg(reg) >> (byte * 8)) & 0xFF) as u8)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        let mut v = self.read_reg(reg);
        let mask: u32 = 0xFF << (byte * 8);
        v = (v & !mask) | ((value as u32) << (byte * 8));
        self.write_reg(reg, v);
        Ok(())
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

/// PWR — STM32L0 layout (RM0367 §6.4).
///
/// The L0 power controller is just two registers — CR @0x00 and CSR @0x04 —
/// not the wide CR1..CR4 / PUCRx surface of the L4 model. Reset values pinned
/// to a NUCLEO-L073RZ SWD probe (2026-06-11): CR = 0x0000_1000 (VOS = Range 2,
/// the power-on default), CSR = 0, and every higher offset reads 0 (the L4
/// PUCRx/PDCRx registers do not exist here). Modeling L0 with the L4 struct
/// made CR read 0x0200 and surfaced phantom CR3/SR2 bits at 0x08/0x14.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct PwrL0 {
    cr: u32,
    csr: u32,
}

impl PwrL0 {
    pub fn new() -> Self {
        Self {
            cr: 0x0000_1000,
            csr: 0,
        }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.csr,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            // CR writable bits [15:0]: LPSDSR, PDDS, CWUF, CSBF, PVDE,
            // PLS[2:0], DBP, ULP, FWU, VOS[1:0], DS_EE_KOFF, LPRUN.
            0x00 => self.cr = value & 0x0000_FFFF,
            // CSR: status is read-only (WUF/SBF clear via CR.CWUF/CSBF);
            // only the EWUP1..3 enables (bits [10:8]) are writable here.
            0x04 => self.csr = (self.csr & !0x0000_0700) | (value & 0x0000_0700),
            _ => {}
        }
    }
}

impl crate::Peripheral for PwrL0 {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        Ok(((self.read_reg(reg) >> (byte * 8)) & 0xFF) as u8)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        let mut v = self.read_reg(reg);
        let mask: u32 = 0xFF << (byte * 8);
        v = (v & !mask) | ((value as u32) << (byte * 8));
        self.write_reg(reg, v);
        Ok(())
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

/// STM32WBA PWR (RM0493). Zephyr's set_regu_voltage writes PWR_VOSR (0x0C) to
/// select the voltage range/EPOD boost, then spins on VOSRDY (bit 15) before the
/// PLL is configured; the SoC init also flips enable bits in other PWR registers
/// and reads them straight back. Arduino-HAL `HAL_PWREx_ControlVoltageScaling`
/// additionally polls **SVMSR** @ 0x3C for **ACTVOSRDY** (bit 15) and compares
/// **ACTVOS** (bit 16) to the requested scale.
///
/// Model:
/// - VOSR writes: VOSRDY (15) always after a program; BOOSTRDY (14) if EPOD (18).
/// - SVMSR reads: synthesize ACTVOS from VOSR.VOS and ACTVOSRDY once VOS is set
///   (or after any VOSR write). Other regs are plain read-back storage.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct PwrWba {
    regs: std::collections::HashMap<u64, u32>,
}

impl PwrWba {
    pub fn new() -> Self {
        Self::default()
    }

    fn read_reg(&self, offset: u64) -> u32 {
        if offset == 0x3C {
            // SVMSR: ACTVOS mirrors VOSR.VOS; ACTVOSRDY follows VOSRDY.
            let vosr = self.regs.get(&0x0C).copied().unwrap_or(0);
            let mut svmsr = self.regs.get(&0x3C).copied().unwrap_or(0);
            // Clear hardware status bits we synthesize, keep software-visible others.
            svmsr &= !((1 << 15) | (1 << 16));
            let vos = vosr & (0x3 << 16);
            if vos != 0 || (vosr & (1 << 15)) != 0 {
                svmsr |= 1 << 15; // ACTVOSRDY
                svmsr |= vos; // ACTVOS[16:17] / ACTVOS bit16 on WBA55
            }
            return svmsr;
        }
        self.regs.get(&offset).copied().unwrap_or(0)
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        let mut v = value;
        if offset == 0x0C {
            // Any VOS program completes instantly: VOSRDY (15).
            // CMSIS: VOS is bit 16 (single); mask [16:17] for forward-compat.
            v |= 1 << 15; // VOSRDY
            if v & (1 << 18) != 0 {
                v |= 1 << 14; // BOOSTRDY with EPOD
            }
        }
        if offset == 0x3C {
            // SVMSR is mostly status; ignore FW writes to ACTVOS/ACTVOSRDY.
            return;
        }
        self.regs.insert(offset, v);
    }
}

impl crate::Peripheral for PwrWba {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        Ok(((self.read_reg(reg) >> (byte * 8)) & 0xFF) as u8)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        let mut v = self.read_reg(reg);
        let mask: u32 = 0xFF << (byte * 8);
        v = (v & !mask) | ((value as u32) << (byte * 8));
        self.write_reg(reg, v);
        Ok(())
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

/// PWR — STM32F4 layout (RM0368 §5.4, STM32F401). Only two registers exist:
/// PWR_CR (0x00) and PWR_CSR (0x04) — none of the L4 CR1..CR4 / PUCRx surface.
/// Reset values pinned to the STM32F401 CMSIS SVD (the reset-conformance
/// oracle): CR = 0x0000_0000, CSR = 0x0000_0000. (RM0368 §5.4.1 prints CR =
/// 0x0000_8000 for the VOS = Scale-2 default, but the ST SVD — and the
/// register_coverage gate built from it — pin CR = 0; VOSRDY is asserted only
/// after firmware programs a scale, below.)
///
/// Foreign firmware (HAL / Zephyr) enables the PWR clock, programs PWR_CR.VOS,
/// then spins on PWR_CSR.VOSRDY (bit 14) before touching the PLL. Voltage
/// scaling completes instantly in the sim: any write to PWR_CR latches VOSRDY,
/// so the reset read still returns 0 (matching silicon) but the boot poll
/// resolves as soon as firmware selects a scale.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct PwrF4 {
    cr: u32,
    csr: u32,
}

impl PwrF4 {
    pub fn new() -> Self {
        Self { cr: 0, csr: 0 }
    }

    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.csr,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            // CR writable bits: VOS[15:14], ADCDC1(13), MRLVDS(11), LPLVDS(10),
            // FPDS(9), DBP(8), PLS[7:5], PVDE(4), PDDS(1), LPDS(0). CSBF(3) and
            // CWUF(2) are write-only strobes that always read 0; bit 12 and
            // [31:16] are reserved. Selecting a voltage scale asserts VOSRDY.
            0x00 => {
                self.cr = value & 0x0000_EFF3;
                self.csr |= 1 << 14; // VOSRDY — scaling completes instantly
            }
            // CSR: EWUP (bit 8) and BRE (bit 9) are the only writable bits;
            // VOSRDY/BRR/PVDO/SBF/WUF are hardware status.
            0x04 => self.csr = (self.csr & !0x0000_0300) | (value & 0x0000_0300),
            _ => {}
        }
    }
}

impl crate::Peripheral for PwrF4 {
    // Inert walk: pure register bank (voltage scaling resolves in the write path).
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        Ok(((self.read_reg(reg) >> (byte * 8)) & 0xFF) as u8)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let byte = (offset % 4) as u32;
        let mut v = self.read_reg(reg);
        let mask: u32 = 0xFF << (byte * 8);
        v = (v & !mask) | ((value as u32) << (byte * 8));
        self.write_reg(reg, v);
        Ok(())
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod f4_tests {
    use super::PwrF4;
    use crate::Peripheral;

    #[test]
    fn pwr_f4_reset_matches_rm0368() {
        let pwr = PwrF4::new();
        // STM32F401 CMSIS SVD (register_coverage oracle).
        assert_eq!(pwr.read_u32(0x00).unwrap(), 0x0000_0000, "PWR_CR reset");
        assert_eq!(pwr.read_u32(0x04).unwrap(), 0x0000_0000, "PWR_CSR reset");
        // The L4 CR3 / SR2 / PUCRx surface must not exist on F4.
        assert_eq!(pwr.read_u32(0x08).unwrap(), 0, "no register at 0x08");
        assert_eq!(pwr.read_u32(0x14).unwrap(), 0, "no register at 0x14");
    }

    #[test]
    fn pwr_f4_vosrdy_sets_on_scale_select() {
        let mut pwr = PwrF4::new();
        // Firmware selects Scale 1 (VOS = 0b11) then polls VOSRDY.
        pwr.write_u32(0x00, 0x0000_C000).unwrap();
        assert_ne!(
            pwr.read_u32(0x04).unwrap() & (1 << 14),
            0,
            "VOSRDY asserted"
        );
    }
}

#[cfg(test)]
mod l0_tests {
    use super::PwrL0;
    use crate::Peripheral;

    #[test]
    fn pwr_l0_reset_matches_silicon() {
        // Silicon capture 2026-06-11 (NUCLEO-L073RZ, ST-Link/V2).
        let pwr = PwrL0::new();
        assert_eq!(pwr.read_u32(0x00).unwrap(), 0x0000_1000, "CR reset");
        assert_eq!(pwr.read_u32(0x04).unwrap(), 0x0000_0000, "CSR reset");
        // The L4 PUCRx/SR surface does not exist on L0 — must read 0.
        assert_eq!(pwr.read_u32(0x08).unwrap(), 0, "no register at 0x08");
        assert_eq!(pwr.read_u32(0x14).unwrap(), 0, "no register at 0x14");
    }
}

#[cfg(test)]
mod wba_tests {
    use super::PwrWba;
    use crate::Peripheral;

    #[test]
    fn vosr_vos_select_acks_vosrdy() {
        let mut pwr = PwrWba::new();
        // set_regu_voltage selects a VOS range (bit16) and spins on VOSRDY (15).
        pwr.write_u32(0x0C, 1 << 16).unwrap();
        assert_ne!(pwr.read_u32(0x0C).unwrap() & (1 << 15), 0, "VOSRDY acked");
        // EPOD boost (bit18) acks BOOSTRDY (bit14).
        pwr.write_u32(0x0C, 1 << 18).unwrap();
        assert_ne!(pwr.read_u32(0x0C).unwrap() & (1 << 14), 0, "BOOSTRDY acked");
    }

    #[test]
    fn svmsr_mirrors_actvos_for_hal_voltage_scaling() {
        // Arduino HAL_PWREx_ControlVoltageScaling polls SVMSR.ACTVOSRDY (bit15)
        // and compares SVMSR.ACTVOS (bit16) to the requested scale.
        let mut pwr = PwrWba::new();
        pwr.write_u32(0x0C, 1 << 16).unwrap();
        let svmsr = pwr.read_u32(0x3C).unwrap();
        assert_ne!(svmsr & (1 << 15), 0, "ACTVOSRDY");
        assert_ne!(svmsr & (1 << 16), 0, "ACTVOS mirrors VOS");
    }

    #[test]
    fn other_regs_are_readback_storage() {
        // The SoC init flips enable bits in other PWR regs and reads them back.
        let mut pwr = PwrWba::new();
        pwr.write_u32(0x28, 1).unwrap();
        assert_eq!(pwr.read_u32(0x28).unwrap(), 1);
    }
}

#[cfg(test)]
mod h5_tests {
    use super::PwrH5;
    use crate::Peripheral;

    #[test]
    fn pwr_h5_reset_and_vos_tracking_match_silicon() {
        // Silicon capture 2026-06-11 (NUCLEO-H563ZI).
        let mut pwr = PwrH5::new();
        assert_eq!(pwr.read_u32(0x00).unwrap(), 0x0000_000C, "PMCR");
        assert_eq!(pwr.read_u32(0x14).unwrap(), 0x0000_2008, "VOSSR reset");
        assert_eq!(pwr.read_u32(0x30).unwrap(), 0x0000_0100, "SCCR");
        assert_eq!(pwr.read_u32(0x3C).unwrap(), 0x0010_0000, "VMSR");

        pwr.write_u32(0x10, 0x30).unwrap(); // VOS = Scale0
        assert_eq!(pwr.read_u32(0x14).unwrap(), 0x0000_E008, "VOSSR scale0");
        pwr.write_u32(0x10, 0x10).unwrap(); // VOS = Scale2
        assert_eq!(pwr.read_u32(0x14).unwrap(), 0x0000_6008, "VOSSR scale2");
        pwr.write_u32(0x10, 0).unwrap();
        assert_eq!(pwr.read_u32(0x14).unwrap(), 0x0000_2008, "VOSSR scale3");

        pwr.write_u32(0x24, 0x1).unwrap(); // DBP
        assert_eq!(pwr.read_u32(0x24).unwrap(), 0x1, "DBPCR");
    }
}
