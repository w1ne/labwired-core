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
    /// STM32H7 family (RM0468 / RM0433 — H723/733/725/735/730, H74x/75x).
    /// Reference-manual-derived; not silicon-verified (no H7 bench part).
    Stm32H7,
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
            "h7" | "stm32h7" => Ok(Self::Stm32H7),
            "stm32l4" | "l4" => Ok(Self::Stm32L4),
            "stm32l0" | "l0" => Ok(Self::Stm32L0),
            _ => Err(format!(
                "unsupported RCC register layout '{}'; supported: stm32f1, stm32f4, stm32v2, stm32h5, stm32h7, stm32l4, stm32l0",
                value
            )),
        }
    }
}

// ── Shared, stateless helpers (shared silicon IP behaviour, never shared state) ─

/// Source-ready SW→SWS gate. The SYSCLK switch only completes (SWS follows SW)
/// once the requested source's CR ready bit is set; until then SWS holds its
/// previous value, exactly as silicon does. `rdy_bits[sw]` is the CR ready-bit
/// index for each SW[1:0] encoding, or `None` for a reserved encoding that
/// never switches. This mirrors the per-source gating the L4/L0/H5 models
/// already apply, and prevents the false-pass where firmware switches SYSCLK to
/// a source it never enabled+waited for.
///
/// Per-family `rdy_bits` (silicon-verified against the ST CMSIS headers):
///   F1/F4 (CFGR @ 0x04/0x08): 00 HSI→1, 01 HSE→17, 10 PLL→25, 11 reserved.
///   G4/WB (CFGR @ 0x08):      00 MSI→1, 01 HSI16→10, 10 HSE→17, 11 PLL→25.
///   WBA   (CFGR1 @ 0x1C):     00 HSI16→10, 01 reserved, 10 HSE→17, 11 PLL1R→25.
fn cfgr_with_gated_sws(value: u32, cr: u32, prev_cfgr: u32, rdy_bits: [Option<u32>; 4]) -> u32 {
    let sw = (value & 0x3) as usize;
    let ready = matches!(rdy_bits[sw], Some(b) if cr & (1 << b) != 0);
    let sws = if ready {
        (value & 0x3) << 2
    } else {
        prev_cfgr & (0x3 << 2)
    };
    (value & !(0x3 << 2)) | sws
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
    bdcr: u32,     // 0x20 — RTC/LSE backup domain control
    csr: u32,      // 0x24 — LSION bit0 → LSIRDY bit1
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
            0x20 => self.bdcr,
            0x24 => self.csr,
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
            // SW→SWS only follows once the requested source is ready in CR:
            // 00 HSI (bit1), 01 HSE (bit17), 10 PLL (bit25), 11 reserved.
            0x04 => {
                self.cfgr = cfgr_with_gated_sws(
                    value,
                    self.cr,
                    self.cfgr,
                    [Some(1), Some(17), Some(25), None],
                )
            }
            0x08 => self.cir = value & 0x0000_1F00,
            0x0C => self.apb2rstr = value,
            0x10 => self.apb1rstr = value,
            0x14 => self.ahbenr = value & 0x0000_0055, // DMA1/SRAM/FLITF/CRC
            0x18 => self.apb2enr = value & 0x0000_5E7D,
            0x1C => self.apb1enr = value & 0x1AE6_4807,
            // BDCR: LSEON (bit0) → LSERDY (bit1); the rest is RTC/backup storage.
            0x20 => {
                self.bdcr = if value & 1 != 0 {
                    value | (1 << 1)
                } else {
                    value & !(1 << 1)
                };
            }
            // CSR: LSION (bit0) → LSIRDY (bit1). The reset-flag bits (31:24) are
            // kept as plain storage. Zephyr's RTC/LSI clock init polls LSIRDY.
            0x24 => {
                self.csr = if value & 1 != 0 {
                    value | (1 << 1)
                } else {
                    value & !(1 << 1)
                };
            }
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
            // CR reset = 0x0000_0083 (RM0368 §6.3.1 / RM0090 §6.3.1): HSION
            // (bit 0), HSIRDY (bit 1, auto), HSITRIM = 0x10 default (bits 7:3 =
            // 0x80). The bare 1<<0 dropped the HSITRIM default and read 0x03.
            cr: classic_cr_ready(0x0000_0083),
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
            // CFGR at 0x08 on F4 (not 0x04). SW→SWS follows only once the
            // requested source is ready in CR: 00 HSI (bit1), 01 HSE (bit17),
            // 10 PLL (bit25), 11 reserved.
            0x08 => {
                self.cfgr = cfgr_with_gated_sws(
                    value,
                    self.cr,
                    self.cfgr,
                    [Some(1), Some(17), Some(25), None],
                )
            }
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
    cfgr: u32,     // 0x08 (G4/WB RM0440/RM0434: CR=0x00, ICSCR=0x04, CFGR=0x08)
    ahbenr: u32,   // AHB2ENR 0x8C
    apb1enr: u32,  // APB1LENR 0x9C
    apb2enr: u32,  // 0xA4
    ahbrstr: u32,  // 0x6C
    apb1rstr: u32, // 0x7C
    apb2rstr: u32, // 0x84
    bdcr: u32,     // 0x90 — WB/G4 backup domain: LSEON bit0 → LSERDY bit1
    csr: u32,      // 0x94 — LSION bit0 → LSIRDY bit1
    crrcr: u32,    // 0x98 — HSI48ON bit0 → HSI48RDY bit1
    bdcr1: u32,    // 0xF0 — WBA backup domain: LSI/LSESYS/LSE2 enable→ready pairs
    cfgr1: u32,    // 0x1C — WBA RCC_CFGR1 (SW→SWS); G4/WB use CFGR at 0x08
    reg28: u32,    // 0x28 — WBA: request bit20 ↔ acknowledge bit22 (deselect)
}

impl V2Rcc {
    fn new() -> Self {
        Self {
            cr: Self::ready(1 << 0),
            ..Default::default()
        }
    }
    /// G4/WB/WBA CR ready rule: the classic HSI(0)/HSE(16)/PLL(24) bits plus
    /// HSI16 at bit8→bit10 (these families gate the kernel clock on HSI16RDY,
    /// e.g. WBA's stm32_clock_control_init).
    fn ready(cr: u32) -> u32 {
        let mut cr = classic_cr_ready(cr);
        if cr & (1 << 8) != 0 {
            cr |= 1 << 10;
        } else {
            cr &= !(1 << 10);
        }
        cr
    }
}

impl RccModel for V2Rcc {
    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr,
            0x08 => self.cfgr,
            0x1C => self.cfgr1,
            // 0x28 acknowledge (bit22) tracks the inverse of request bit20:
            // the SoC init clears bit20 and waits for bit22 to confirm.
            0x28 => {
                if self.reg28 & (1 << 20) == 0 {
                    self.reg28 | (1 << 22)
                } else {
                    self.reg28 & !(1 << 22)
                }
            }
            0x6C => self.ahbrstr,
            0x7C => self.apb1rstr,
            0x84 => self.apb2rstr,
            0x8C => self.ahbenr,
            0x90 => self.bdcr,
            0x94 => self.csr,
            0x98 => self.crrcr,
            0x9C => self.apb1enr,
            0xA4 => self.apb2enr,
            0xF0 => self.bdcr1,
            _ => 0,
        }
    }
    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.cr = Self::ready(value),
            // G4/WB RCC_CFGR (0x08): SW→SWS follows only once the requested
            // source is ready in CR — 00 MSI (bit1), 01 HSI16 (bit10),
            // 10 HSE (bit17), 11 PLL (bit25).
            0x08 => {
                self.cfgr = cfgr_with_gated_sws(
                    value,
                    self.cr,
                    self.cfgr,
                    [Some(1), Some(10), Some(17), Some(25)],
                )
            }
            // WBA RCC_CFGR1 (0x1C): SW[1:0]→SWS[3:2], gated on the source's CR
            // ready bit — 00 HSI16 (bit10), 01 reserved, 10 HSE (bit17),
            // 11 PLL1R (bit25). (RM0493: SW=01 is not a defined source.)
            0x1C => {
                self.cfgr1 = cfgr_with_gated_sws(
                    value,
                    self.cr,
                    self.cfgr1,
                    [Some(10), None, Some(17), Some(25)],
                )
            }
            0x28 => self.reg28 = value,
            // BDCR: LSEON (bit0) → LSERDY (bit1); rest is RTC/backup storage.
            0x90 => {
                self.bdcr = if value & 1 != 0 {
                    value | (1 << 1)
                } else {
                    value & !(1 << 1)
                };
            }
            0x6C => self.ahbrstr = value,
            0x7C => self.apb1rstr = value,
            0x84 => self.apb2rstr = value,
            0x8C => self.ahbenr = value,
            // CSR: LSION (bit0) → LSIRDY (bit1); reset flags (31:23) are storage.
            0x94 => {
                self.csr = if value & 1 != 0 {
                    value | (1 << 1)
                } else {
                    value & !(1 << 1)
                };
            }
            // CRRCR: HSI48ON (bit0) → HSI48RDY (bit1).
            0x98 => {
                self.crrcr = if value & 1 != 0 {
                    value | (1 << 1)
                } else {
                    value & !(1 << 1)
                };
            }
            0x9C => self.apb1enr = value,
            0xA4 => self.apb2enr = value,
            // BDCR1 (WBA backup domain, RM0493): the LSI / LSESYS / LSE2
            // enable→ready handshakes Zephyr's clock init polls — LSION(0)→
            // LSIRDY(1), LSESYSEN(7)→LSESYSRDY(11), and the bit26→bit27 pair.
            // Other bits (LSE config, RTC sel) are plain storage.
            0xF0 => {
                let mut v = value;
                for &(on, rdy) in &[(0u32, 1u32), (7, 11), (26, 27)] {
                    if v & (1 << on) != 0 {
                        v |= 1 << rdy;
                    } else {
                        v &= !(1 << rdy);
                    }
                }
                self.bdcr1 = v;
            }
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

// ── STM32H7 ─────────────────────────────────────────────────────────────────
// Register offsets per RM0468 (STM32H723/733/725/735/730) / RM0433. The
// enable-register block (0xD4..0xF4), BDCR (0x70) and CSR (0x74) are identical
// across the H7 line; the domain/PLL/CCIPR configuration registers are modelled
// as plain read/write storage so HAL read-modify-write bring-up round-trips.
//
// NOT silicon-verified: LabWired has no H735 bench part. Reset values are
// reference-manual-derived (RM0468 §8.7). The live behaviour that firmware
// depends on — oscillator/PLL ready gating in CR, source-ready-gated SYSCLK
// switch (SW→SWS) in CFGR, LSE/LSI ready in BDCR/CSR, and clock-enable
// round-trip — is modelled; clock *frequencies* are not.
#[derive(Debug, serde::Serialize)]
pub struct H7Rcc {
    cr: u32,        // 0x00
    hsicfgr: u32,   // 0x04
    crrcr: u32,     // 0x08
    csicfgr: u32,   // 0x0C
    cfgr: u32,      // 0x10 — SW[2:0] → SWS[5:3]
    d1cfgr: u32,    // 0x18 (RM0468: CDCFGR1)
    d2cfgr: u32,    // 0x1C (CDCFGR2)
    d3cfgr: u32,    // 0x20 (SRDCFGR)
    pllckselr: u32, // 0x28 — reset 0x02020200
    pllcfgr: u32,   // 0x2C — reset 0x01FF0000
    pll1divr: u32,  // 0x30 — reset 0x01010280
    pll1fracr: u32, // 0x34
    pll2divr: u32,  // 0x38 — reset 0x01010280
    pll2fracr: u32, // 0x3C
    pll3divr: u32,  // 0x40 — reset 0x01010280
    pll3fracr: u32, // 0x44
    d1ccipr: u32,   // 0x4C (CDCCIPR)
    d2ccip1r: u32,  // 0x50 (CDCCIP1R)
    d2ccip2r: u32,  // 0x54 (CDCCIP2R)
    d3ccipr: u32,   // 0x58 (SRDCCIPR)
    cier: u32,      // 0x60
    cifr: u32,      // 0x64
    cicr: u32,      // 0x68
    bdcr: u32,      // 0x70 — LSE/RTC backup domain
    csr: u32,       // 0x74 — LSI
    ahb3rstr: u32,  // 0x7C
    ahb1rstr: u32,  // 0x80
    ahb2rstr: u32,  // 0x84
    ahb4rstr: u32,  // 0x88
    apb3rstr: u32,  // 0x8C
    apb1lrstr: u32, // 0x90
    apb1hrstr: u32, // 0x94
    apb2rstr: u32,  // 0x98
    apb4rstr: u32,  // 0x9C
    rsr: u32,       // 0xD0 — reset-cause flags
    ahb3enr: u32,   // 0xD4
    ahb1enr: u32,   // 0xD8
    ahb2enr: u32,   // 0xDC
    ahb4enr: u32,   // 0xE0
    apb3enr: u32,   // 0xE4
    apb1lenr: u32,  // 0xE8
    apb1henr: u32,  // 0xEC
    apb2enr: u32,   // 0xF0
    apb4enr: u32,   // 0xF4
}

/// H7 CR ready rule (RM0468 §8.7.2): each oscillator/PLL ON bit auto-sets its
/// RDY bit — HSI 0→2, CSI 7→8, HSI48 12→13, HSE 16→17, PLL1 24→25, PLL2 26→27,
/// PLL3 28→29. HSIDIVF (bit 5) tracks HSION (divider update instantaneous in
/// the model), and the domain-clock-ready bits D1CKRDY/D2CKRDY (14/15) read
/// ready so HAL bring-up that polls them proceeds.
fn h7_cr_ready(mut cr: u32) -> u32 {
    for &(on, rdy) in &[
        (0u32, 2u32),
        (7, 8),
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
    cr | (1 << 14) | (1 << 15)
}

impl H7Rcc {
    fn new() -> Self {
        Self {
            cr: h7_cr_ready(0x0000_0001), // HSION → HSIRDY|HSIDIVF (= 0x25)
            hsicfgr: 0x4000_0000,
            crrcr: 0,
            csicfgr: 0x2000_0000,
            cfgr: 0,
            d1cfgr: 0,
            d2cfgr: 0,
            d3cfgr: 0,
            pllckselr: 0x0202_0200,
            pllcfgr: 0x01FF_0000,
            pll1divr: 0x0101_0280,
            pll1fracr: 0,
            pll2divr: 0x0101_0280,
            pll2fracr: 0,
            pll3divr: 0x0101_0280,
            pll3fracr: 0,
            d1ccipr: 0,
            d2ccip1r: 0,
            d2ccip2r: 0,
            d3ccipr: 0,
            cier: 0,
            cifr: 0,
            cicr: 0,
            bdcr: 0,
            csr: 0,
            ahb3rstr: 0,
            ahb1rstr: 0,
            ahb2rstr: 0,
            ahb4rstr: 0,
            apb3rstr: 0,
            apb1lrstr: 0,
            apb1hrstr: 0,
            apb2rstr: 0,
            apb4rstr: 0,
            rsr: 0,
            ahb3enr: 0,
            ahb1enr: 0,
            ahb2enr: 0,
            ahb4enr: 0,
            apb3enr: 0,
            apb1lenr: 0,
            apb1henr: 0,
            apb2enr: 0,
            apb4enr: 0,
        }
    }
}

impl RccModel for H7Rcc {
    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.hsicfgr,
            0x08 => self.crrcr,
            0x0C => self.csicfgr,
            0x10 => self.cfgr,
            0x18 => self.d1cfgr,
            0x1C => self.d2cfgr,
            0x20 => self.d3cfgr,
            0x28 => self.pllckselr,
            0x2C => self.pllcfgr,
            0x30 => self.pll1divr,
            0x34 => self.pll1fracr,
            0x38 => self.pll2divr,
            0x3C => self.pll2fracr,
            0x40 => self.pll3divr,
            0x44 => self.pll3fracr,
            0x4C => self.d1ccipr,
            0x50 => self.d2ccip1r,
            0x54 => self.d2ccip2r,
            0x58 => self.d3ccipr,
            0x60 => self.cier,
            0x64 => self.cifr,
            0x68 => self.cicr,
            0x70 => self.bdcr,
            0x74 => self.csr,
            0x7C => self.ahb3rstr,
            0x80 => self.ahb1rstr,
            0x84 => self.ahb2rstr,
            0x88 => self.ahb4rstr,
            0x8C => self.apb3rstr,
            0x90 => self.apb1lrstr,
            0x94 => self.apb1hrstr,
            0x98 => self.apb2rstr,
            0x9C => self.apb4rstr,
            0xD0 => self.rsr,
            0xD4 => self.ahb3enr,
            0xD8 => self.ahb1enr,
            0xDC => self.ahb2enr,
            0xE0 => self.ahb4enr,
            0xE4 => self.apb3enr,
            0xE8 => self.apb1lenr,
            0xEC => self.apb1henr,
            0xF0 => self.apb2enr,
            0xF4 => self.apb4enr,
            _ => 0,
        }
    }
    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.cr = h7_cr_ready(value),
            0x04 => self.hsicfgr = value,
            0x08 => self.crrcr = value,
            0x0C => self.csicfgr = value,
            // SW[2:0] → SWS[5:3] only when the requested source is ready in CR
            // (source→RDY bit: HSI→2, CSI→8, HSE→17, PLL1→25). Mirrors the H5
            // gate so firmware that switches SYSCLK to an un-readied source
            // never sees the switch complete.
            0x10 => {
                let sw = value & 0x7;
                let ready = match sw {
                    0 => self.cr & (1 << 2) != 0,
                    1 => self.cr & (1 << 8) != 0,
                    2 => self.cr & (1 << 17) != 0,
                    3 => self.cr & (1 << 25) != 0,
                    _ => false,
                };
                let sws = if ready {
                    sw << 3
                } else {
                    self.cfgr & (0x7 << 3)
                };
                self.cfgr = (value & !(0x7 << 3)) | sws;
            }
            0x18 => self.d1cfgr = value,
            0x1C => self.d2cfgr = value,
            0x20 => self.d3cfgr = value,
            0x28 => self.pllckselr = value,
            0x2C => self.pllcfgr = value,
            0x30 => self.pll1divr = value,
            0x34 => self.pll1fracr = value,
            0x38 => self.pll2divr = value,
            0x3C => self.pll2fracr = value,
            0x40 => self.pll3divr = value,
            0x44 => self.pll3fracr = value,
            0x4C => self.d1ccipr = value,
            0x50 => self.d2ccip1r = value,
            0x54 => self.d2ccip2r = value,
            0x58 => self.d3ccipr = value,
            0x60 => self.cier = value,
            0x64 => self.cifr = value,
            // CICR is write-1-to-clear against CIFR; model the ack.
            0x68 => {
                self.cifr &= !value;
                self.cicr = 0;
            }
            // BDCR ready rule: LSEON bit0 → LSERDY bit1 (RM0468 §8.7.28).
            0x70 => {
                let mut bdcr = value;
                if bdcr & 1 != 0 {
                    bdcr |= 1 << 1;
                } else {
                    bdcr &= !(1 << 1);
                }
                self.bdcr = bdcr;
            }
            // CSR: LSION bit0 → LSIRDY bit1 (RM0468 §8.7.29).
            0x74 => {
                let mut csr = value;
                if csr & 1 != 0 {
                    csr |= 1 << 1;
                } else {
                    csr &= !(1 << 1);
                }
                self.csr = csr;
            }
            0x7C => self.ahb3rstr = value,
            0x80 => self.ahb1rstr = value,
            0x84 => self.ahb2rstr = value,
            0x88 => self.ahb4rstr = value,
            0x8C => self.apb3rstr = value,
            0x90 => self.apb1lrstr = value,
            0x94 => self.apb1hrstr = value,
            0x98 => self.apb2rstr = value,
            0x9C => self.apb4rstr = value,
            0xD0 => self.rsr = value,
            0xD4 => self.ahb3enr = value,
            0xD8 => self.ahb1enr = value,
            0xDC => self.ahb2enr = value,
            0xE0 => self.ahb4enr = value,
            0xE4 => self.apb3enr = value,
            0xE8 => self.apb1lenr = value,
            0xEC => self.apb1henr = value,
            0xF0 => self.apb2enr = value,
            0xF4 => self.apb4enr = value,
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
    bdcr: u32,     // 0x90 — LSE/RTC backup domain control
    csr: u32,      // 0x94 — LSION bit0 → LSIRDY bit1
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
    /// L4 CR ready rule: MSI bit0→bit1; HSI16 bit8→bit10; HSE bit16→bit17 gated
    /// by HSEBYP(bit18); PLL bit24→bit25, PLLSAI1 bit26→bit27, PLLSAI2 bit28→bit29
    /// all gated by the PLLCFGR.PLLSRC clock being ready. (Zephyr's
    /// LL_RCC_HSI_IsReady polls HSIRDY at bit10; STM32 HAL polls PLLSAI1RDY.)
    fn ready(&self, mut cr: u32) -> u32 {
        if cr & (1 << 0) != 0 {
            cr |= 1 << 1;
        } else {
            cr &= !(1 << 1);
        }
        if cr & (1 << 8) != 0 {
            cr |= 1 << 10;
        } else {
            cr &= !(1 << 10);
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
            2 => cr & (1 << 10) != 0, // HSI16
            3 => cr & (1 << 17) != 0, // HSE
            _ => false,
        };
        if cr & (1 << 24) != 0 && src_ready {
            cr |= 1 << 25;
        } else {
            cr &= !(1 << 25);
        }
        // FIDELITY: modeled, NOT HW-validated (2026-07-04) — RCC_CR.PLLSAI1RDY
        // (bit27) follows PLLSAI1ON (bit26); RCC_CR.PLLSAI2RDY (bit29) follows
        // PLLSAI2ON (bit28). RM0351 §6.4.1 (RCC_CR): each PLLSAIxON enable sets
        // its RDY flag once the PLL locks. The SAI PLLs share the main PLL input
        // clock (RCC_PLLCFGR.PLLSRC bits[1:0]), so they can only lock when that
        // source is ready — gate on src_ready exactly like the main PLL above.
        // STM32 HAL's RCCEx_PLLSAI1_Config spins on PLLSAI1RDY after setting
        // PLLSAI1ON (Arduino STM32 core enables PLLSAI1 for the 48 MHz domain);
        // without this the poll never exits and boot hangs before first print.
        if cr & (1 << 26) != 0 && src_ready {
            cr |= 1 << 27;
        } else {
            cr &= !(1 << 27);
        }
        if cr & (1 << 28) != 0 && src_ready {
            cr |= 1 << 29;
        } else {
            cr &= !(1 << 29);
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
            0x90 => self.bdcr,
            0x94 => self.csr,
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
                let hsirdy = self.cr & (1 << 10) != 0;
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
            // BDCR: LSEON (bit0) → LSERDY (bit1); rest is RTC/backup storage.
            0x90 => {
                self.bdcr = if value & 1 != 0 {
                    value | (1 << 1)
                } else {
                    value & !(1 << 1)
                };
            }
            // CSR: LSION (bit0) → LSIRDY (bit1); reset flags (31:23) are storage.
            0x94 => {
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
    csr: u32,      // 0x50 — LSION bit0 → LSIRDY bit1
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
            0x50 => self.csr,
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
            // CSR: LSION (bit0) → LSIRDY (bit1); reset flags (31:23) are storage.
            0x50 => {
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

// ── Dispatcher ──────────────────────────────────────────────────────────────

/// RCC peripheral — one variant per chip family. Each variant's registers are
/// fully isolated; no register from one family exists on another.
#[derive(Debug)]
pub enum Rcc {
    Stm32F1(F1Rcc),
    Stm32F4(F4Rcc),
    Stm32V2(V2Rcc),
    Stm32H5(H5Rcc),
    Stm32H7(H7Rcc),
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
            RccRegisterLayout::Stm32H7 => Self::Stm32H7(H7Rcc::new()),
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
            // H7: AHB3ENR@0xD4, AHB1ENR@0xD8, AHB2ENR@0xDC, AHB4ENR@0xE0,
            // APB3ENR@0xE4, APB1LENR@0xE8, APB1HENR@0xEC, APB2ENR@0xF0,
            // APB4ENR@0xF4 (RM0468 §8.7). The enable block is identical across
            // the H7 line.
            Self::Stm32H7(_) => match r.as_str() {
                "ahb3enr" => Some(0xD4),
                "ahb1enr" | "ahbenr" => Some(0xD8),
                "ahb2enr" => Some(0xDC),
                "ahb4enr" => Some(0xE0),
                "apb3enr" => Some(0xE4),
                "apb1enr" | "apb1lenr" => Some(0xE8),
                "apb1henr" => Some(0xEC),
                "apb2enr" => Some(0xF0),
                "apb4enr" => Some(0xF4),
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
            Self::Stm32H7(r) => r,
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
            Self::Stm32H7(r) => r,
            Self::Stm32L4(r) => r,
            Self::Stm32L0(r) => r,
        }
    }
}

impl crate::Peripheral for Rcc {
    // Inert walk: clock-control register bank; tick() is the trait-default no-op.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

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

    /// Every family Zephyr drives auto-acks LSION→LSIRDY in its CSR, at the
    /// per-family offset. Zephyr's RTC/LSI clock init spins on LSIRDY.
    #[test]
    fn lsi_ready_auto_acks_per_family() {
        for (layout, csr) in [
            (RccRegisterLayout::Stm32F1, 0x24u64),
            (RccRegisterLayout::Stm32F4, 0x74),
            (RccRegisterLayout::Stm32L0, 0x50),
            (RccRegisterLayout::Stm32L4, 0x94),
            (RccRegisterLayout::Stm32V2, 0x94),
        ] {
            let mut rcc = Rcc::new_with_layout(layout);
            rcc.write_u32(csr, 1).unwrap(); // LSION
            assert_eq!(
                rcc.read_u32(csr).unwrap() & 0x3,
                0x3,
                "{:?} CSR@{:#x} must set LSIRDY",
                layout,
                csr
            );
        }
    }

    /// L4 and the G4/WB/WBA V2 layout gate the kernel clock on HSI16RDY, which
    /// hardware reports at CR bit 10 (HSION at bit 8), not the MSI bit-1 slot.
    #[test]
    fn hsi16_ready_at_bit10() {
        for layout in [RccRegisterLayout::Stm32L4, RccRegisterLayout::Stm32V2] {
            let mut rcc = Rcc::new_with_layout(layout);
            let cr = rcc.read_u32(0x00).unwrap();
            rcc.write_u32(0x00, cr | (1 << 8)).unwrap(); // HSION
            assert_ne!(
                rcc.read_u32(0x00).unwrap() & (1 << 10),
                0,
                "{:?} must set HSIRDY at bit 10",
                layout
            );
        }
    }

    /// G4/WB/WBA expose HSI48 via CRRCR (0x98): HSI48ON→HSI48RDY.
    #[test]
    fn v2_hsi48_ready() {
        let mut rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32V2);
        rcc.write_u32(0x98, 1).unwrap();
        assert_eq!(rcc.read_u32(0x98).unwrap() & 0x3, 0x3);
    }

    /// WB's classic RCC_BDCR (0x90) acks LSEON→LSERDY; WBA's BDCR1 (0xF0) acks
    /// LSION(0)→LSIRDY(1), LSESYSEN(7)→LSESYSRDY(11) and the bit26→bit27 pair;
    /// WBA RCC_CFGR1 (0x1C) follows SW→SWS; RCC 0x28 acks the bit20→bit22 deselect.
    #[test]
    fn v2_wb_wba_backup_and_switch_gates() {
        let mut rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32V2);

        rcc.write_u32(0x90, 1).unwrap(); // BDCR LSEON
        assert_eq!(rcc.read_u32(0x90).unwrap() & 0x3, 0x3, "LSERDY");

        rcc.write_u32(0xF0, (1 << 0) | (1 << 7) | (1 << 26))
            .unwrap();
        let bdcr1 = rcc.read_u32(0xF0).unwrap();
        for rdy in [1u32, 11, 27] {
            assert_ne!(bdcr1 & (1 << rdy), 0, "BDCR1 rdy bit {rdy}");
        }

        // CFGR1 SW=PLL1R (0b11) is gated on PLLRDY: with the PLL off the switch
        // holds; enabling PLL1 (CR bit24 → PLLRDY bit25) lets it complete.
        rcc.write_u32(0x1C, 0x3).unwrap();
        assert_eq!(
            (rcc.read_u32(0x1C).unwrap() >> 2) & 0x3,
            0x0,
            "SWS holds while PLL not ready"
        );
        rcc.write_u32(0x00, 1 << 24).unwrap(); // PLL1ON → PLL1RDY
        rcc.write_u32(0x1C, 0x3).unwrap();
        assert_eq!(
            (rcc.read_u32(0x1C).unwrap() >> 2) & 0x3,
            0x3,
            "SWS follows SW once PLL ready"
        );

        // 0x28: clearing request bit20 confirms via ack bit22.
        rcc.write_u32(0x28, 0).unwrap();
        assert_ne!(rcc.read_u32(0x28).unwrap() & (1 << 22), 0, "deselect ack");
        rcc.write_u32(0x28, 1 << 20).unwrap();
        assert_eq!(
            rcc.read_u32(0x28).unwrap() & (1 << 22),
            0,
            "ack clears with request"
        );
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
    fn test_rcc_cfgr_sws_follows_sw_when_source_ready() {
        let mut rcc = Rcc::new();
        // SW=PLL (0b10) with PLL off: the switch must NOT complete — SWS holds.
        rcc.write_u32(0x04, 0b10).unwrap();
        let cfgr = rcc.read_u32(0x04).unwrap();
        assert_eq!(cfgr & 0b11, 0b10); // SW latched
        assert_eq!((cfgr >> 2) & 0b11, 0b00, "SWS holds while PLL not ready");
        // Enable PLL (CR bit24 → PLLRDY bit25), then SW=PLL completes.
        rcc.write_u32(0x00, rcc.read_u32(0x00).unwrap() | (1 << 24))
            .unwrap();
        rcc.write_u32(0x04, 0b10).unwrap();
        assert_eq!((rcc.read_u32(0x04).unwrap() >> 2) & 0b11, 0b10, "SWS=PLL");
    }

    #[test]
    fn test_rcc_l4_sai_pll_ready_flags_follow_enable_bits() {
        // RM0351 §6.4.1: PLLSAI1ON (bit26)→PLLSAI1RDY (bit27) and PLLSAI2ON
        // (bit28)→PLLSAI2RDY (bit29), gated on the shared PLL input source.
        let mut rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32L4);
        // At reset PLLCFGR.PLLSRC=0 (no clock): enabling PLLSAI1 must NOT lock.
        rcc.write_u32(0x00, rcc.read_u32(0x00).unwrap() | (1 << 26))
            .unwrap();
        assert_eq!(
            rcc.read_u32(0x00).unwrap() & (1 << 27),
            0,
            "PLLSAI1RDY stays clear while PLL source is not ready"
        );
        // Select MSI (PLLSRC=01) as the PLL input — MSI is ready at reset.
        rcc.write_u32(0x0C, 0x1).unwrap();
        rcc.write_u32(0x00, rcc.read_u32(0x00).unwrap() | (1 << 26) | (1 << 28))
            .unwrap();
        let cr = rcc.read_u32(0x00).unwrap();
        assert_ne!(cr & (1 << 27), 0, "PLLSAI1RDY set once source ready");
        assert_ne!(cr & (1 << 29), 0, "PLLSAI2RDY set once source ready");
        // Clearing the enable clears the ready flag.
        rcc.write_u32(0x00, cr & !(1 << 26)).unwrap();
        assert_eq!(
            rcc.read_u32(0x00).unwrap() & (1 << 27),
            0,
            "PLLSAI1RDY clears with PLLSAI1ON=0"
        );
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
