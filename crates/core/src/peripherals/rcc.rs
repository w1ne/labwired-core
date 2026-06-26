// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// ── Architectural separation ────────────────────────────────────────────────
// RCC is modelled as one struct PER CHIP FAMILY (F1 / F4 / V2 / L4 / L0),
// unified by the `Rcc` enum. Each family struct owns ONLY the registers that
// family actually has — so e.g. the L0-only CRRCR/IOPENR registers physically
// cannot exist on an F4 or L4 instance, and a change to one family's model
// cannot leak into another. The chip yaml's `profile` selects the variant via
// `RccRegisterLayout`; the `Peripheral` impl dispatches to the active family.
//
// Shared *behaviour* (not state) lives in small stateless helper fns
// (`classic_cr_ready`, etc.) where families genuinely share silicon IP.

use crate::SimResult;
use std::str::FromStr;

/// Selects which chip family's RCC model to instantiate. Kept as the public
/// config-facing selector (chip yaml `profile`); each value maps 1:1 to a
/// dedicated family struct below.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RccRegisterLayout {
    #[default]
    Stm32F1,
    Stm32F4,
    Stm32V2,
    /// STM32H5 family (RM0481). Register offsets and reset values verified on
    /// NUCLEO-H563ZI silicon over SWD (`scripts/hw-capture-stm32h563.sh`).
    Stm32H5,
    /// STM32L4 family (RM0351). Verified on NUCLEO-L476RG over SWD.
    Stm32L4,
    /// STM32L0 family (RM0367). Verified on NUCLEO-L073RZ over SWD.
    Stm32L0,
}

impl FromStr for RccRegisterLayout {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let v = value.trim().to_ascii_lowercase();
        match v.as_str() {
            "stm32f1" | "f1" | "legacy" => Ok(Self::Stm32F1),
            "stm32f4" | "f4" => Ok(Self::Stm32F4),
            "stm32v2" | "v2" | "modern" | "stm32-modern" => Ok(Self::Stm32V2),
            "h5" | "stm32h5" => Ok(Self::Stm32H5),
            "stm32l4" | "l4" => Ok(Self::Stm32L4),
            "stm32l0" | "l0" => Ok(Self::Stm32L0),
            _ => Err(format!(
                "unsupported RCC register layout '{}'; supported: stm32f1, stm32f4, stm32v2, stm32h5, stm32l4, stm32l0",
                value
            )),
        }
    }
}

// ── Shared, stateless helpers (shared silicon IP behaviour, never shared state) ─

/// Optimistic SW→SWS: the switch completes immediately (SWS mirrors SW).
/// Used by the classic families (F1/F4/V2) whose existing models assume the
/// requested source is always ready.
fn cfgr_with_optimistic_sws(value: u32) -> u32 {
    (value & !(0x3 << 2)) | ((value & 0x3) << 2)
}

/// Classic CR ready-flag rule (F1/F4/V2): each ON bit auto-sets its RDY bit.
///   HSION bit0 → HSIRDY bit1, HSEON bit16 → HSERDY bit17, PLLON bit24 → PLLRDY bit25.
fn classic_cr_ready(mut cr: u32) -> u32 {
    for &(on, rdy) in &[(0u32, 1u32), (16, 17), (24, 25)] {
        if cr & (1 << on) != 0 {
            cr |= 1 << rdy;
        } else {
            cr &= !(1 << rdy);
        }
    }
    cr
}

/// Internal per-family register model. Implemented by each family struct.
trait RccModel: std::fmt::Debug {
    fn read_reg(&self, offset: u64) -> u32;
    fn write_reg(&mut self, offset: u64, value: u32);
    fn snapshot(&self) -> serde_json::Value;
}

// ── STM32F1 ─────────────────────────────────────────────────────────────────
#[derive(Debug, Default, serde::Serialize)]
pub struct F1Rcc {
    cr: u32,
    cfgr: u32,     // 0x04
    cir: u32,      // 0x08
    ahbenr: u32,   // 0x14
    apb2enr: u32,  // 0x18
    apb1enr: u32,  // 0x1C
    apb2rstr: u32, // 0x0C
    apb1rstr: u32, // 0x10
    ahbrstr: u32,  // 0x28
}

impl F1Rcc {
    fn new() -> Self {
        // CR reset verified on real STM32F103C8 silicon (Blue Pill): 0x00004A83
        //   bit0 HSION=1, bit1 HSIRDY=1, bits7:3 HSITRIM=0x10 (default trim),
        //   bits15:8 HSICAL=0x4A (chip calibration). classic_cr_ready is a no-op
        //   here (HSIRDY already set, no HSE/PLL).
        Self {
            cr: classic_cr_ready(0x0000_4A83),
            // AHBENR reset = 0x14 (SRAMEN bit2 + FLITFEN bit4 enabled out of
            // reset). Silicon-verified on the bench STM32F103: a read-back of
            // RCC_AHBENR after ORing CRCEN returned 0x54 = 0x14 | (1<<6)
            // (stm32f1_exec_oracle::crc32_two_words). RM0008 §7.3.6.
            ahbenr: 0x0000_0014,
            ..Default::default()
        }
    }
}

impl RccModel for F1Rcc {
    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.cfgr,
            0x08 => self.cir,
            0x0C => self.apb2rstr,
            0x10 => self.apb1rstr,
            0x14 => self.ahbenr,
            0x18 => self.apb2enr,
            0x1C => self.apb1enr,
            0x28 => self.ahbrstr,
            _ => 0,
        }
    }
    fn write_reg(&mut self, offset: u64, value: u32) {
        // ENR / CIR writable masks silicon-confirmed on the bench F103 via the
        // address sweep. F103 is the only F1 chip, so these are exact (no other
        // density shares F1Rcc). The clear/flag bits of CIR (write-only 23:16,
        // read-only flags 7:0) carry no persistent state — only the interrupt-
        // enable bits 12:8 (0x1F00) read back.
        match offset {
            0x00 => self.cr = classic_cr_ready(value),
            0x04 => self.cfgr = cfgr_with_optimistic_sws(value),
            0x08 => self.cir = value & 0x0000_1F00,
            0x0C => self.apb2rstr = value,
            0x10 => self.apb1rstr = value,
            0x14 => self.ahbenr = value & 0x0000_0055, // DMA1/SRAM/FLITF/CRC
            0x18 => self.apb2enr = value & 0x0000_5E7D,
            0x1C => self.apb1enr = value & 0x1AE6_4807,
            0x28 => self.ahbrstr = value,
            _ => {}
        }
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

// ── STM32F4 ─────────────────────────────────────────────────────────────────
// F4 RCC register map silicon-confirmed on the bench F407 (RM0090 §6.3):
// PLLCFGR@0x04, CFGR@0x08, CIR@0x0C, AHB1ENR@0x30, AHB2ENR@0x34, APB1/2ENR@0x40/44.
// The clock-enable (ENR) writable masks are PER-PART (which peripherals the
// device physically has) — F4Rcc is shared with the smaller F401 — so they are
// per-instance fields set from the chip config (default 0xFFFF_FFFF = unmasked,
// so an un-pinned part keeps the permissive behaviour). F407's masks are filled
// from `configs/chips/stm32f407.yaml`; F401's stay default until benched.
#[derive(Debug, Default, serde::Serialize)]
pub struct F4Rcc {
    cr: u32,
    pllcfgr: u32,  // 0x04
    cfgr: u32,     // 0x08
    cir: u32,      // 0x0C
    ahbenr: u32,   // AHB1ENR 0x30
    ahb2enr: u32,  // 0x34
    apb1enr: u32,  // 0x40
    apb2enr: u32,  // 0x44
    ahbrstr: u32,  // AHB1RSTR 0x10
    apb1rstr: u32, // 0x20
    apb2rstr: u32, // 0x24
    csr: u32,      // 0x74 — LSION bit0 → LSIRDY bit1
    // Per-part ENR writable masks (silicon-pinned); 0xFFFF_FFFF = unmasked.
    ahb1_mask: u32,
    apb1_mask: u32,
    apb2_mask: u32,
}

impl F4Rcc {
    fn new() -> Self {
        Self {
            cr: classic_cr_ready(1 << 0),
            // PLLCFGR reset = 0x24003010 (RM0090 §6.3.2) — the factory default
            // PLL config word; firmware reads it back before reconfiguring.
            pllcfgr: 0x2400_3010,
            ahb1_mask: 0xFFFF_FFFF,
            apb1_mask: 0xFFFF_FFFF,
            apb2_mask: 0xFFFF_FFFF,
            ..Default::default()
        }
    }
}

impl RccModel for F4Rcc {
    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.pllcfgr,
            0x08 => self.cfgr,
            0x0C => self.cir,
            0x10 => self.ahbrstr,
            0x20 => self.apb1rstr,
            0x24 => self.apb2rstr,
            0x30 => self.ahbenr,
            0x34 => self.ahb2enr,
            0x40 => self.apb1enr,
            0x44 => self.apb2enr,
            0x74 => self.csr,
            _ => 0,
        }
    }
    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.cr = classic_cr_ready(value),
            // PLLCFGR writable = PLLM/PLLN/PLLP/PLLSRC/PLLQ = 0x7F43_7FFF
            // (silicon-confirmed on F407). Reserved bits read 0.
            0x04 => self.pllcfgr = value & 0x7F43_7FFF,
            // CFGR at 0x08 on F4 (not 0x04). The optimistic SW->SWS fake lets
            // firmware that switches SYSCLK to PLL/HSE see the switch complete.
            0x08 => self.cfgr = cfgr_with_optimistic_sws(value),
            // CIR interrupt-enable bits 13:8 = 0x3F00 (F4 adds PLLI2SRDYIE bit
            // 13 over the F1's 5 bits) — silicon-confirmed on F407.
            0x0C => self.cir = value & 0x0000_3F00,
            0x10 => self.ahbrstr = value,
            0x20 => self.apb1rstr = value,
            0x24 => self.apb2rstr = value,
            // ENR writable bits = the part's implemented peripherals (per-part
            // mask, silicon-pinned). AHB2ENR unmasked for now (OTG/RNG/etc.).
            0x30 => self.ahbenr = value & self.ahb1_mask,
            0x34 => self.ahb2enr = value,
            0x40 => self.apb1enr = value & self.apb1_mask,
            0x44 => self.apb2enr = value & self.apb2_mask,
            // CSR: LSION (bit0) auto-sets LSIRDY (bit1), mirroring the classic
            // CR ready rule. The reset-flag bits (25:31) and RMVF (24) are
            // kept as plain storage.
            0x74 => {
                self.csr = if value & 1 != 0 {
                    value | (1 << 1)
                } else {
                    value & !(1 << 1)
                };
            }
            _ => {}
        }
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

// ── STM32V2 (H5-style) ──────────────────────────────────────────────────────
#[derive(Debug, Default, serde::Serialize)]
pub struct V2Rcc {
    cr: u32,
    cfgr: u32,     // 0x04
    ahbenr: u32,   // AHB2ENR 0x8C
    apb1enr: u32,  // APB1LENR 0x9C
    apb2enr: u32,  // 0xA4
    ahbrstr: u32,  // 0x6C
    apb1rstr: u32, // 0x7C
    apb2rstr: u32, // 0x84
}

impl V2Rcc {
    fn new() -> Self {
        Self {
            cr: classic_cr_ready(1 << 0),
            ..Default::default()
        }
    }
}

impl RccModel for V2Rcc {
    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.cfgr,
            0x6C => self.ahbrstr,
            0x7C => self.apb1rstr,
            0x84 => self.apb2rstr,
            0x8C => self.ahbenr,
            0x9C => self.apb1enr,
            0xA4 => self.apb2enr,
            _ => 0,
        }
    }
    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.cr = classic_cr_ready(value),
            0x04 => self.cfgr = cfgr_with_optimistic_sws(value),
            0x6C => self.ahbrstr = value,
            0x7C => self.apb1rstr = value,
            0x84 => self.apb2rstr = value,
            0x8C => self.ahbenr = value,
            0x9C => self.apb1enr = value,
            0xA4 => self.apb2enr = value,
            _ => {}
        }
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

// ── STM32H5 ─────────────────────────────────────────────────────────────────
// Register offsets per RM0481; every reset value below was captured from a
// NUCLEO-H563ZI (DBGMCU_IDCODE 0x10016484, Cortex-M33 r0p4) at reset halt via
// `scripts/hw-capture-stm32h563.sh` on 2026-06-10.
#[derive(Debug, serde::Serialize)]
pub struct H5Rcc {
    cr: u32,           // 0x00 — reset 0x0000002B (HSION|HSIRDY|HSIDIV=÷2|HSIDIVF)
    hsicfgr: u32,      // 0x10 — reset 0x004004F7 (HSITRIM=0x40, HSICAL factory)
    csicfgr: u32,      // 0x18 — reset 0x00200087 (CSITRIM=0x20, CSICAL factory)
    cfgr1: u32,        // 0x1C — SW[2:0] → SWS[5:3]
    cfgr2: u32,        // 0x20
    pllcfgr: [u32; 3], // 0x28 / 0x2C / 0x30
    ahb1rstr: u32,     // 0x60
    ahb2rstr: u32,     // 0x64
    apb1lrstr: u32,    // 0x74
    apb1hrstr: u32,    // 0x78
    apb2rstr: u32,     // 0x7C
    apb3rstr: u32,     // 0x80
    ahb1enr: u32,      // 0x88 — reset 0xD0000100
    ahb2enr: u32,      // 0x8C — reset 0xC0000000 (SRAM2EN|SRAM3EN)
    apb1lenr: u32,     // 0x9C
    apb1henr: u32,     // 0xA0
    apb2enr: u32,      // 0xA4
    apb3enr: u32,      // 0xA8
    bdcr: u32,         // 0xF0
    rsr: u32,          // 0xF4 — reset 0x0C000000 (PINRST|BORRST)
}

impl H5Rcc {
    fn new() -> Self {
        Self {
            cr: h5_cr_ready(0x0000_0029),
            hsicfgr: 0x0040_04F7,
            csicfgr: 0x0020_0087,
            cfgr1: 0,
            cfgr2: 0,
            pllcfgr: [0; 3],
            ahb1rstr: 0,
            ahb2rstr: 0,
            apb1lrstr: 0,
            apb1hrstr: 0,
            apb2rstr: 0,
            apb3rstr: 0,
            ahb1enr: 0xD000_0100,
            ahb2enr: 0xC000_0000,
            apb1lenr: 0,
            apb1henr: 0,
            apb2enr: 0,
            apb3enr: 0,
            bdcr: 0,
            rsr: 0x0C00_0000,
        }
    }
}

/// H5 CR ready rule: each oscillator/PLL ON bit auto-sets its RDY bit —
/// HSI 0→1, CSI 8→9, HSI48 12→13, HSE 16→17, PLL1 24→25, PLL2 26→27,
/// PLL3 28→29. HSIDIVF (bit 5) tracks HSION: the divider update is
/// instantaneous in the model.
fn h5_cr_ready(mut cr: u32) -> u32 {
    for &(on, rdy) in &[
        (0u32, 1u32),
        (8, 9),
        (12, 13),
        (16, 17),
        (24, 25),
        (26, 27),
        (28, 29),
    ] {
        if cr & (1 << on) != 0 {
            cr |= 1 << rdy;
        } else {
            cr &= !(1 << rdy);
        }
    }
    if cr & 1 != 0 {
        cr |= 1 << 5;
    } else {
        cr &= !(1 << 5);
    }
    cr
}

impl RccModel for H5Rcc {
    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr,
            0x10 => self.hsicfgr,
            0x18 => self.csicfgr,
            0x1C => self.cfgr1,
            0x20 => self.cfgr2,
            0x28 => self.pllcfgr[0],
            0x2C => self.pllcfgr[1],
            0x30 => self.pllcfgr[2],
            0x60 => self.ahb1rstr,
            0x64 => self.ahb2rstr,
            0x74 => self.apb1lrstr,
            0x78 => self.apb1hrstr,
            0x7C => self.apb2rstr,
            0x80 => self.apb3rstr,
            0x88 => self.ahb1enr,
            0x8C => self.ahb2enr,
            0x9C => self.apb1lenr,
            0xA0 => self.apb1henr,
            0xA4 => self.apb2enr,
            0xA8 => self.apb3enr,
            0xF0 => self.bdcr,
            0xF4 => self.rsr,
            _ => 0,
        }
    }
    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.cr = h5_cr_ready(value),
            // HSICFGR / CSICFGR: TRIM is the only writable field, and CAL
            // tracks it linearly — silicon-probed on the bench H563: HSITRIM
            // 0x40→0x55 moved HSICAL 0x4F7→0x50C (+0x15), CSITRIM 0x20→0x15
            // moved CSICAL 0x87→0x7C (-0xB). CAL base values are this part's
            // factory calibration at the default trim.
            0x10 => {
                let trim = (value >> 16) & 0x7F;
                self.hsicfgr = (trim << 16) | ((0x4F7 + trim - 0x40) & 0xFFF);
            }
            0x18 => {
                let trim = (value >> 16) & 0x3F;
                self.csicfgr = (trim << 16) | ((0x87 + trim - 0x20) & 0xFF);
            }
            // SW[2:0] → SWS[5:3] only when the requested source is ready in
            // CR (silicon-probed: SW=CSI with CSI off leaves SWS unchanged;
            // setting CSION first completes the switch). Source→RDY bit:
            // HSI→1, CSI→9, HSE→17, PLL1→25.
            0x1C => {
                let sw = value & 0x7;
                let ready = match sw {
                    0 => self.cr & (1 << 1) != 0,
                    1 => self.cr & (1 << 9) != 0,
                    2 => self.cr & (1 << 17) != 0,
                    3 => self.cr & (1 << 25) != 0,
                    _ => false, // reserved encodings never switch
                };
                let sws = if ready {
                    sw << 3
                } else {
                    self.cfgr1 & (0x7 << 3)
                };
                self.cfgr1 = (value & !(0x7 << 3)) | sws;
            }
            0x20 => self.cfgr2 = value,
            0x28 => self.pllcfgr[0] = value,
            0x2C => self.pllcfgr[1] = value,
            0x30 => self.pllcfgr[2] = value,
            0x60 => self.ahb1rstr = value,
            0x64 => self.ahb2rstr = value,
            0x74 => self.apb1lrstr = value,
            0x78 => self.apb1hrstr = value,
            0x7C => self.apb2rstr = value,
            0x80 => self.apb3rstr = value,
            0x88 => self.ahb1enr = value,
            0x8C => self.ahb2enr = value,
            0x9C => self.apb1lenr = value,
            0xA0 => self.apb1henr = value,
            0xA4 => self.apb2enr = value,
            0xA8 => self.apb3enr = value,
            // BDCR ready rule mirrors CR: LSEON bit0 → LSERDY bit1,
            // LSION bit26 → LSIRDY bit27 (RM0481 §11.8.41).
            0xF0 => {
                let mut bdcr = value;
                for (on, rdy) in [(0u32, 1u32), (26, 27)] {
                    if bdcr & (1 << on) != 0 {
                        bdcr |= 1 << rdy;
                    } else {
                        bdcr &= !(1 << rdy);
                    }
                }
                self.bdcr = bdcr;
            }
            // RSR: reset-cause flags are hardware-set; software write only
            // clears them via RMVF (bit 23, silicon-probed) — other writes
            // fall through to the no-op default.
            0xF4 if value & (1 << 23) != 0 => self.rsr = 0,
            _ => {}
        }
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

// ── STM32L4 ─────────────────────────────────────────────────────────────────
#[derive(Debug, Default, serde::Serialize)]
pub struct L4Rcc {
    cr: u32,
    cfgr: u32,     // 0x08
    pllcfgr: u32,  // 0x0C
    ahbenr: u32,   // AHB2ENR 0x4C (GPIO ports)
    apb1enr: u32,  // APB1ENR1 0x58
    apb2enr: u32,  // 0x60
    ahbrstr: u32,  // AHB2RSTR 0x2C
    apb1rstr: u32, // APB1RSTR1 0x38
    apb2rstr: u32, // 0x40
}

impl L4Rcc {
    fn new() -> Self {
        // L4 boots on MSI range 6 (4 MHz): MSION|MSIRDY|MSIRANGE=0b0110 = 0x63.
        let mut s = Self {
            cr: 0x0000_0063,
            ..Default::default()
        };
        s.cr = s.ready(s.cr);
        s
    }
    /// L4 CR ready rule: MSI bit0→bit1; HSE bit16→bit17 gated by HSEBYP(bit18);
    /// PLL bit24→bit25 gated by the PLLCFGR.PLLSRC clock being ready.
    fn ready(&self, mut cr: u32) -> u32 {
        if cr & (1 << 0) != 0 {
            cr |= 1 << 1;
        } else {
            cr &= !(1 << 1);
        }
        let hsebyp = cr & (1 << 18) != 0;
        if cr & (1 << 16) != 0 && hsebyp {
            cr |= 1 << 17;
        } else {
            cr &= !(1 << 17);
        }
        let src = self.pllcfgr & 0x3;
        let src_ready = match src {
            1 => cr & (1 << 1) != 0,  // MSI
            2 => cr & (1 << 1) != 0,  // HSI16 (modelled at bit1, as before)
            3 => cr & (1 << 17) != 0, // HSE
            _ => false,
        };
        if cr & (1 << 24) != 0 && src_ready {
            cr |= 1 << 25;
        } else {
            cr &= !(1 << 25);
        }
        cr
    }
}

impl RccModel for L4Rcc {
    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr,
            0x08 => self.cfgr,
            0x0C => self.pllcfgr,
            0x2C => self.ahbrstr,
            0x38 => self.apb1rstr,
            0x40 => self.apb2rstr,
            0x4C => self.ahbenr,
            0x58 => self.apb1enr,
            0x60 => self.apb2enr,
            _ => 0,
        }
    }
    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.cr = self.ready(value),
            0x08 => {
                // SW→SWS only follows once the requested source is ready.
                let prev_sws = (self.cfgr >> 2) & 0x3;
                let sw = value & 0x3;
                let msirdy = self.cr & (1 << 1) != 0;
                let hsirdy = self.cr & (1 << 1) != 0;
                let hserdy = self.cr & (1 << 17) != 0;
                let pllrdy = self.cr & (1 << 25) != 0;
                let sws = match sw {
                    0 if msirdy => sw,
                    1 if hsirdy => sw,
                    2 if hserdy => sw,
                    3 if pllrdy => sw,
                    _ => prev_sws,
                };
                self.cfgr = (value & !(0x3 << 2)) | (sws << 2);
            }
            0x0C => {
                self.pllcfgr = value;
                self.cr = self.ready(self.cr); // PLLSRC change can re-gate PLLRDY
            }
            0x2C => self.ahbrstr = value,
            0x38 => self.apb1rstr = value,
            0x40 => self.apb2rstr = value,
            0x4C => self.ahbenr = value,
            0x58 => self.apb1enr = value,
            0x60 => self.apb2enr = value,
            _ => {}
        }
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

// ── STM32L0 ─────────────────────────────────────────────────────────────────
// L0-only registers (CRRCR, IOPENR) live HERE and nowhere else.
#[derive(Debug, Default, serde::Serialize)]
pub struct L0Rcc {
    cr: u32,
    crrcr: u32,    // 0x08 — HSI48
    cfgr: u32,     // 0x0C
    ahbrstr: u32,  // 0x20
    apb2rstr: u32, // 0x24
    apb1rstr: u32, // 0x28
    iopenr: u32,   // 0x2C — GPIO port clock enable
    ahbenr: u32,   // 0x30 — DMA/CRC/RNG
    apb2enr: u32,  // 0x34
    apb1enr: u32,  // 0x38
}

impl L0Rcc {
    fn new() -> Self {
        // L0 boots on MSI: CR reset = MSION(bit8)|MSIRDY(bit9) = 0x300.
        let mut s = Self {
            cr: 0x0000_0300,
            ..Default::default()
        };
        s.cr = Self::ready(s.cr);
        s
    }
    /// L0 CR ready rule: HSI16 bit0→bit2, MSI bit8→bit9, HSE bit16→bit17,
    /// PLL bit24→bit25.
    fn ready(mut cr: u32) -> u32 {
        for &(on, rdy) in &[(0u32, 2u32), (8, 9), (16, 17), (24, 25)] {
            if cr & (1 << on) != 0 {
                cr |= 1 << rdy;
            } else {
                cr &= !(1 << rdy);
            }
        }
        cr
    }
}

impl RccModel for L0Rcc {
    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr,
            0x08 => self.crrcr,
            0x0C => self.cfgr,
            0x20 => self.ahbrstr,
            0x24 => self.apb2rstr,
            0x28 => self.apb1rstr,
            0x2C => self.iopenr,
            0x30 => self.ahbenr,
            0x34 => self.apb2enr,
            0x38 => self.apb1enr,
            _ => 0,
        }
    }
    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.cr = Self::ready(value),
            0x08 => {
                // CRRCR.HSI48ON (bit0) → HSI48RDY (bit1).
                self.crrcr = if value & 1 != 0 {
                    value | (1 << 1)
                } else {
                    value & !(1 << 1)
                };
            }
            0x0C => {
                // SW→SWS gated by the L0 CR ready bits (MSIRDY bit9,
                // HSI16RDY bit2, HSERDY bit17, PLLRDY bit25).
                let prev_sws = (self.cfgr >> 2) & 0x3;
                let sw = value & 0x3;
                let msirdy = self.cr & (1 << 9) != 0;
                let hsi16rdy = self.cr & (1 << 2) != 0;
                let hserdy = self.cr & (1 << 17) != 0;
                let pllrdy = self.cr & (1 << 25) != 0;
                let sws = match sw {
                    0 if msirdy => sw,
                    1 if hsi16rdy => sw,
                    2 if hserdy => sw,
                    3 if pllrdy => sw,
                    _ => prev_sws,
                };
                self.cfgr = (value & !(0x3 << 2)) | (sws << 2);
            }
            0x20 => self.ahbrstr = value,
            0x24 => self.apb2rstr = value,
            0x28 => self.apb1rstr = value,
            0x2C => self.iopenr = value,
            0x30 => self.ahbenr = value,
            0x34 => self.apb2enr = value,
            0x38 => self.apb1enr = value,
            _ => {}
        }
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

// ── Dispatcher ──────────────────────────────────────────────────────────────

/// RCC peripheral — one variant per chip family. Each variant's registers are
/// fully isolated; no register from one family exists on another.
#[derive(Debug)]
pub enum Rcc {
    Stm32F1(F1Rcc),
    Stm32F4(F4Rcc),
    Stm32V2(V2Rcc),
    Stm32H5(H5Rcc),
    Stm32L4(L4Rcc),
    Stm32L0(L0Rcc),
}

impl Default for Rcc {
    fn default() -> Self {
        Self::Stm32F1(F1Rcc::new())
    }
}

impl Rcc {
    pub fn new() -> Self {
        Self::new_with_layout(RccRegisterLayout::Stm32F1)
    }

    pub fn new_with_layout(layout: RccRegisterLayout) -> Self {
        match layout {
            RccRegisterLayout::Stm32F1 => Self::Stm32F1(F1Rcc::new()),
            RccRegisterLayout::Stm32F4 => Self::Stm32F4(F4Rcc::new()),
            RccRegisterLayout::Stm32V2 => Self::Stm32V2(V2Rcc::new()),
            RccRegisterLayout::Stm32H5 => Self::Stm32H5(H5Rcc::new()),
            RccRegisterLayout::Stm32L4 => Self::Stm32L4(L4Rcc::new()),
            RccRegisterLayout::Stm32L0 => Self::Stm32L0(L0Rcc::new()),
        }
    }

    /// Resolve a symbolic clock-enable register name (e.g. "apb1enr",
    /// "apb2enr", "ahbenr", "ahb2enr") to its byte offset within THIS chip
    /// family's RCC register map. Returns `None` for an unknown name on the
    /// active family. The offsets deliberately differ between families
    /// (F1 apb1enr@0x1C vs L4 apb1enr@0x58), which is exactly why this lives
    /// on the family-aware model rather than in the bus.
    ///
    /// Used by the bus to map a peripheral's `clock: { reg, bit }` declaration
    /// onto the real RCC register it must read for the gate check.
    pub fn enable_reg_offset(&self, reg: &str) -> Option<u64> {
        let r = reg.trim().to_ascii_lowercase();
        match self {
            // F1: AHBENR@0x14, APB2ENR@0x18, APB1ENR@0x1C (RM0008 §7.3).
            Self::Stm32F1(_) => match r.as_str() {
                "ahbenr" | "ahb1enr" => Some(0x14),
                "apb2enr" => Some(0x18),
                "apb1enr" | "apb1enr1" => Some(0x1C),
                _ => None,
            },
            // F4: AHB1ENR@0x30, AHB2ENR@0x34, APB1ENR@0x40, APB2ENR@0x44.
            Self::Stm32F4(_) => match r.as_str() {
                "ahbenr" | "ahb1enr" => Some(0x30),
                "ahb2enr" => Some(0x34),
                "apb1enr" | "apb1enr1" => Some(0x40),
                "apb2enr" => Some(0x44),
                _ => None,
            },
            // L4: AHB2ENR@0x4C, APB1ENR1@0x58, APB2ENR@0x60 (RM0351 §6.4).
            Self::Stm32L4(_) => match r.as_str() {
                "ahbenr" | "ahb2enr" => Some(0x4C),
                "apb1enr" | "apb1enr1" => Some(0x58),
                "apb2enr" => Some(0x60),
                _ => None,
            },
            // L0: IOPENR@0x2C, AHBENR@0x30, APB2ENR@0x34, APB1ENR@0x38.
            Self::Stm32L0(_) => match r.as_str() {
                "iopenr" => Some(0x2C),
                "ahbenr" => Some(0x30),
                "apb2enr" => Some(0x34),
                "apb1enr" => Some(0x38),
                _ => None,
            },
            // V2 (H5-style): AHB2ENR@0x8C, APB1LENR@0x9C, APB2ENR@0xA4.
            Self::Stm32V2(_) => match r.as_str() {
                "ahbenr" | "ahb2enr" => Some(0x8C),
                "apb1enr" | "apb1lenr" => Some(0x9C),
                "apb2enr" => Some(0xA4),
                _ => None,
            },
            // H5: AHB1ENR@0x88, AHB2ENR@0x8C, APB1LENR@0x9C, APB1HENR@0xA0,
            // APB2ENR@0xA4, APB3ENR@0xA8 (RM0481 §11.8).
            Self::Stm32H5(_) => match r.as_str() {
                "ahb1enr" | "ahbenr" => Some(0x88),
                "ahb2enr" => Some(0x8C),
                "apb1enr" | "apb1lenr" => Some(0x9C),
                "apb1henr" => Some(0xA0),
                "apb2enr" => Some(0xA4),
                "apb3enr" => Some(0xA8),
                _ => None,
            },
        }
    }

    /// Set the F4 clock-enable (ENR) writable masks — the per-part delta (which
    /// peripherals the device has). No-op for non-F4 layouts. `0xFFFF_FFFF`
    /// leaves a register unmasked.
    pub fn set_f4_enr_masks(&mut self, ahb1: u32, apb1: u32, apb2: u32) {
        if let Self::Stm32F4(r) = self {
            r.ahb1_mask = ahb1;
            r.apb1_mask = apb1;
            r.apb2_mask = apb2;
        }
    }

    fn model(&self) -> &dyn RccModel {
        match self {
            Self::Stm32F1(r) => r,
            Self::Stm32F4(r) => r,
            Self::Stm32V2(r) => r,
            Self::Stm32H5(r) => r,
            Self::Stm32L4(r) => r,
            Self::Stm32L0(r) => r,
        }
    }

    fn model_mut(&mut self) -> &mut dyn RccModel {
        match self {
            Self::Stm32F1(r) => r,
            Self::Stm32F4(r) => r,
            Self::Stm32V2(r) => r,
            Self::Stm32H5(r) => r,
            Self::Stm32L4(r) => r,
            Self::Stm32L0(r) => r,
        }
    }
}

impl crate::Peripheral for Rcc {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = self.model().read_reg(reg_offset);
        Ok(((reg_val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let mut reg_val = self.model().read_reg(reg_offset);

        let mask = 0xFF << (byte_offset * 8);
        reg_val &= !mask;
        reg_val |= (value as u32) << (byte_offset * 8);

        self.model_mut().write_reg(reg_offset, reg_val);
        Ok(())
    }

    fn snapshot(&self) -> serde_json::Value {
        self.model().snapshot()
    }

    // Exposed so the bus can resolve a peripheral's symbolic clock-gate register
    // name to a concrete offset via the family-aware `enable_reg_offset`.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::{Rcc, RccRegisterLayout};
    use crate::Peripheral;

    #[test]
    fn test_rcc_f1_offsets() {
        // Offset round-trip with mask-valid bits (the ENR writable masks are
        // silicon-pinned: AHBENR 0x55, APB2ENR 0x5E7D, APB1ENR 0x1AE64807).
        let mut rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32F1);
        rcc.write(0x14, 0x11).unwrap(); // AHBENR: DMA1EN|FLITFEN (in 0x55)
        rcc.write(0x18, 0x04).unwrap(); // APB2ENR: IOPAEN bit2 (in 0x5E7D)
        rcc.write(0x1C, 0x01).unwrap(); // APB1ENR: TIM2EN bit0 (in 0x1AE64807)
        assert_eq!(rcc.read(0x14).unwrap(), 0x11);
        assert_eq!(rcc.read(0x18).unwrap(), 0x04);
        assert_eq!(rcc.read(0x1C).unwrap(), 0x01);
    }

    #[test]
    fn test_rcc_f4_offsets() {
        let mut rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32F4);
        rcc.write(0x30, 0x12).unwrap(); // AHB1ENR
        rcc.write(0x44, 0x34).unwrap(); // APB2ENR
        rcc.write(0x40, 0x56).unwrap(); // APB1ENR
        assert_eq!(rcc.read(0x30).unwrap(), 0x12);
        assert_eq!(rcc.read(0x44).unwrap(), 0x34);
        assert_eq!(rcc.read(0x40).unwrap(), 0x56);
    }

    #[test]
    fn test_rcc_h5_reset_values() {
        // Reset values captured from NUCLEO-H563ZI silicon at reset halt
        // (scripts/hw-capture-stm32h563.sh, 2026-06-10).
        let rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32H5);
        assert_eq!(rcc.read_u32(0x00).unwrap(), 0x0000_002B); // CR
        assert_eq!(rcc.read_u32(0x10).unwrap(), 0x0040_04F7); // HSICFGR
        assert_eq!(rcc.read_u32(0x18).unwrap(), 0x0020_0087); // CSICFGR
        assert_eq!(rcc.read_u32(0x1C).unwrap(), 0x0000_0000); // CFGR1
        assert_eq!(rcc.read_u32(0x88).unwrap(), 0xD000_0100); // AHB1ENR
        assert_eq!(rcc.read_u32(0x8C).unwrap(), 0xC000_0000); // AHB2ENR
        assert_eq!(rcc.read_u32(0xF4).unwrap(), 0x0C00_0000); // RSR
    }

    #[test]
    fn test_rcc_h5_behaviour() {
        let mut rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32H5);
        // HSEON (bit 16) latches HSERDY (bit 17); dropping it clears RDY.
        let cr = rcc.read_u32(0x00).unwrap();
        rcc.write_u32(0x00, cr | (1 << 16)).unwrap();
        assert_ne!(rcc.read_u32(0x00).unwrap() & (1 << 17), 0);
        rcc.write_u32(0x00, cr).unwrap();
        assert_eq!(rcc.read_u32(0x00).unwrap() & (1 << 17), 0);
        // CFGR1: SW→SWS is gated on the source's CR ready bit. Silicon-probed:
        // SW=CSI with CSI off leaves SWS at the current source; CSION first
        // completes the switch.
        rcc.write_u32(0x1C, 0x1).unwrap();
        assert_eq!((rcc.read_u32(0x1C).unwrap() >> 3) & 0x7, 0x0, "CSI off");
        let cr = rcc.read_u32(0x00).unwrap();
        rcc.write_u32(0x00, cr | (1 << 8)).unwrap(); // CSION → CSIRDY
        rcc.write_u32(0x1C, 0x1).unwrap();
        assert_eq!(rcc.read_u32(0x1C).unwrap(), 0x9, "CSI ready → SWS=001");
        rcc.write_u32(0x1C, 0x0).unwrap();
        rcc.write_u32(0x00, cr).unwrap();
        // HSICFGR: HSITRIM writable, HSICAL tracks trim linearly
        // (silicon-probed: trim 0x55 → cal 0x50C on the bench part).
        rcc.write_u32(0x10, 0x0055_0000).unwrap();
        assert_eq!(rcc.read_u32(0x10).unwrap(), 0x0055_050C);
        rcc.write_u32(0x10, 0x0040_0000).unwrap();
        assert_eq!(rcc.read_u32(0x10).unwrap(), 0x0040_04F7);
        // RSR: flags clear only via RMVF (bit 23, silicon-probed — bit 16
        // writes are ignored).
        rcc.write_u32(0xF4, 1 << 16).unwrap();
        assert_eq!(rcc.read_u32(0xF4).unwrap(), 0x0C00_0000);
        rcc.write_u32(0xF4, 1 << 23).unwrap();
        assert_eq!(rcc.read_u32(0xF4).unwrap(), 0);
        // APB1HENR / APB3ENR round-trip at H5 offsets.
        rcc.write_u32(0xA0, 0x0000_0020).unwrap();
        assert_eq!(rcc.read_u32(0xA0).unwrap(), 0x0000_0020);
        rcc.write_u32(0xA8, 0x0020_0840).unwrap();
        assert_eq!(rcc.read_u32(0xA8).unwrap(), 0x0020_0840);
        // BDCR: LSION (bit 26) latches LSIRDY (bit 27), dropped on clear.
        rcc.write_u32(0xF0, 1 << 26).unwrap();
        assert_ne!(rcc.read_u32(0xF0).unwrap() & (1 << 27), 0);
        rcc.write_u32(0xF0, 0).unwrap();
        assert_eq!(rcc.read_u32(0xF0).unwrap(), 0);
    }

    #[test]
    fn test_rcc_v2_offsets() {
        let mut rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32V2);
        rcc.write(0x8C, 0xF0).unwrap(); // AHB2ENR
        rcc.write(0xA4, 0xCC).unwrap();
        rcc.write(0x9C, 0x33).unwrap();
        assert_eq!(rcc.read(0x8C).unwrap(), 0xF0);
        assert_eq!(rcc.read(0xA4).unwrap(), 0xCC);
        assert_eq!(rcc.read(0x9C).unwrap(), 0x33);
        assert_eq!(rcc.read(0x18).unwrap(), 0x00);
    }

    #[test]
    fn test_rcc_cr_ready_flags_follow_enable_bits() {
        let mut rcc = Rcc::new();
        assert_eq!(rcc.read(0x00).unwrap() & 0x02, 0x02); // HSIRDY set at reset

        rcc.write(0x00, 0x00).unwrap();
        assert_eq!(rcc.read(0x00).unwrap() & 0x02, 0x00); // HSIRDY clears with HSION=0

        // Enable HSE (bit 16) and PLL (bit 24). RDY bits should follow.
        rcc.write(0x02, 0x01).unwrap(); // byte containing bit16
        rcc.write(0x03, 0x01).unwrap(); // byte containing bit24

        let cr_b2 = rcc.read(0x02).unwrap(); // bits 16..23
        let cr_b3 = rcc.read(0x03).unwrap(); // bits 24..31
        assert_eq!(cr_b2 & 0x02, 0x02); // HSERDY (bit17)
        assert_eq!(cr_b3 & 0x02, 0x02); // PLLRDY (bit25)
    }

    #[test]
    fn test_rcc_cfgr_sws_mirrors_sw() {
        let mut rcc = Rcc::new();
        rcc.write(0x04, 0b10).unwrap();
        let cfgr = rcc.read(0x04).unwrap();
        assert_eq!(cfgr & 0b11, 0b10); // SW
        assert_eq!((cfgr >> 2) & 0b11, 0b10); // SWS mirrors SW
    }

    #[test]
    fn test_rcc_l0_layout_and_clock_switch() {
        // Verified against NUCLEO-L073RZ silicon (SWD).
        let mut rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32L0);
        // CR reset = MSION|MSIRDY = 0x300.
        let cr_lo = rcc.read(0x00).unwrap();
        let cr_b1 = rcc.read(0x01).unwrap();
        assert_eq!(cr_lo, 0x00); // bits 0..7
        assert_eq!(cr_b1, 0x03); // bits 8..15 -> MSION(8)+MSIRDY(9)

        // Enable HSI16 (CR bit0); HSI16RDY (bit2) must follow.
        rcc.write(0x00, 0x01).unwrap();
        assert_eq!(rcc.read(0x00).unwrap() & 0x04, 0x04); // HSI16RDY = bit2

        // Switch SYSCLK to HSI16 via CFGR @ 0x0C; SWS must mirror SW=01.
        rcc.write(0x0C, 0x01).unwrap();
        let cfgr = rcc.read(0x0C).unwrap();
        assert_eq!(cfgr & 0b11, 0b01); // SW = HSI16
        assert_eq!((cfgr >> 2) & 0b11, 0b01); // SWS follows -> CLK readback 0x04

        // ENR offsets are L0-specific (APB1ENR @ 0x38, AHBENR @ 0x30).
        rcc.write(0x38, 0xAB).unwrap();
        rcc.write(0x30, 0xCD).unwrap();
        assert_eq!(rcc.read(0x38).unwrap(), 0xAB);
        assert_eq!(rcc.read(0x30).unwrap(), 0xCD);

        // HSI48 (CRRCR @ 0x08): HSI48ON -> HSI48RDY.
        rcc.write(0x08, 0x01).unwrap();
        assert_eq!(rcc.read(0x08).unwrap() & 0x03, 0x03);
    }
}
