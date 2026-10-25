// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// ── Architectural separation ────────────────────────────────────────────────
// RCC is modelled as one struct PER CHIP FAMILY (F1 / F4 / V2 / L4 / L0, and
// the later H5/H7/G4 additions), unified by the `Rcc` enum. Each family struct
// owns ONLY the registers that family actually has — so e.g. the L0-only
// CRRCR/IOPENR registers physically cannot exist on an F4 or L4 instance, and a
// change to one family's model cannot leak into another. `V2Rcc` is the one
// deliberate exception: WB, WBA and U5 share enough of the V2 register file to
// be one struct, with the family deltas — WB's enable-block placement, U5's
// remapped clock tree, WBA's PLL1CFGR ready status — selected by
// fixed-configuration flags each set only by its own constructor and pinned by
// its own tests. The chip yaml's `profile` selects the variant via
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
    /// STM32U5 family (RM0456). Used ONLY by `stm32u575` (`V2Rcc::new_u5`):
    /// the full U5 enable/reset block (AHB1ENR@0x88, AHB2ENR1@0x8C,
    /// AHB2ENR2@0x90, AHB3ENR@0x94, APB1ENR1@0x9C, APB1ENR2@0xA0,
    /// APB2ENR@0xA4, APB3ENR@0xA8 and the eight RSTRs at 0x60..0x80) with the
    /// U5 clock tree — ICSCR1/2/3 @0x08/0x0C/0x10, CRRCR@0x14, CFGR1@0x1C,
    /// CFGR2/3 @0x20/0x24, PLL1/2/3 blocks @0x28..0x48, CIER/CIFR/CICR
    /// @0x50..0x58, BDCR@0xF0, CSR@0xF4 — and U5 CR semantics (MSISRDY bit2,
    /// HSI48 in CR bits 12/13, PLL2/3 ready at 26/27 and 28/29). WB
    /// (`Stm32Wb`), WBA (`Stm32Wba`) and G4 (`Stm32G4`) have their own
    /// layouts; the classic G4/WB-style clock tree lives in the
    /// `!u5_cr_ready` arms of `V2Rcc` for no shipped part.
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
    /// STM32G4 family (RM0440). The modern-V2 clock tree (CR/CFGR/PLLCFGR/
    /// BDCR/CSR/CRRCR) with the RM0440 enable/reset register offsets
    /// (APB1ENR1@0x58, AHB2ENR@0x4C, APB2ENR@0x60) — which differ from the
    /// H5-style V2 layout (APB1LENR@0x9C). Offsets verified against the
    /// vendored CMSIS `stm32g474xx.h`.
    Stm32G4,
    /// STM32WB family (RM0434). Shares the whole modern-V2 clock tree with
    /// WBA/H5 — CR/ICSCR/CFGR/PLLCFGR/BDCR@0x90/CSR@0x94/CRRCR@0x98/
    /// EXTCFGR@0x108 are identical — but places its enable/reset registers in
    /// the L4-shaped block (AHB2ENR@0x4C, APB1ENR1@0x58, APB2ENR@0x60), NOT
    /// the H5-style block at 0x8C/0x9C/0xA4. Offsets from the vendored
    /// `tests/fixtures/real_world/stm32wb55.svd` and ST's CMSIS
    /// `stm32wb55xx.h`. On WB, 0x9C is RCC_HSECR — a real register with
    /// unrelated semantics — so the H5 placement did not merely miss the
    /// enable bits, it aliased them onto silicon that means something else.
    Stm32Wb,
    /// STM32WBA family (RM0493). Same H5-style V2 map as [`Self::Stm32V2`]
    /// (WBA's enable/reset block is at 0x8C/0x9C/0xA4), but `RCC_PLL1CFGR`
    /// bit22 is the read-only `PLL1RCLKPRERDY` status tracking bit20
    /// `PLL1RCLKPRE`. Zephyr's WBA clock init and the Cube HAL clear bit20
    /// and poll bit22; U5 (RM0456) has no such field, so the two must not
    /// share a layout.
    Stm32Wba,
    /// STM32G0 family (RM0444 §5). Its own map: CR/ICSCR/CFGR/PLLCFGR at
    /// 0x00–0x0C with 3-bit SW/SWS fields in CFGR (SW[2:0] bits 0–2,
    /// SWS[2:0] bits 3–5), the clock-interrupt block at 0x18–0x20, and the
    /// RM0444 reset/enable block — IOPRSTR@0x24, AHBRSTR@0x28,
    /// APBRSTR1/2@0x2C/0x30, IOPENR@0x34, AHBENR@0x38, APBENR1@0x3C,
    /// APBENR2@0x40, IOPSMENR@0x44, CCIPR@0x54, BDCR@0x5C, CSR@0x60.
    /// Offsets and CR bit positions verified against ST's CMSIS
    /// `stm32g071xx.h` RCC_TypeDef (fetched from STMicroelectronics/
    /// cmsis-device-g0, 2026-09-19) and RM0444 §5.4; CR reset value
    /// 0x0000_0500 (HSION|HSIRDY) per RM0444 §5.4.1 "Power-on reset value:
    /// 0x0000 0500". G071 has no HSI48/CRRCR (the header defines no
    /// HSI48ON/CRRCR for this part). NOT the L0 map — L0 has CRRCR at 0x08,
    /// a 2-bit CFGR switch, and IOPENR@0x2C.
    Stm32G0,
    /// STM32F0 family (RM0360 §6.4, STM32F030x4/x6/x8/xC and F070). The F1
    /// register map for CR/CFGR/CIR/APB2RSTR/APB1RSTR/AHBENR/APB2ENR/APB1ENR/
    /// BDCR/CSR at 0x00–0x24 — with the same CR enable→ready pairs and the
    /// same 2-bit SW[1:0]/SWS[3:2] switch in CFGR — plus AHBRSTR@0x28,
    /// CFGR2@0x2C (PREDIV), CFGR3@0x30 and CR2@0x34 (HSI14ON bit0 →
    /// HSI14RDY bit1, the ADC's dedicated 14 MHz oscillator; HSI48ON bit16 →
    /// HSI48RDY bit17 on the F04x/F07x/F09x). Unlike [`Self::Stm32F1`] the
    /// enable registers carry the F0 bit positions — the GPIO port clocks are
    /// AHBENR IOPAEN..IOPFEN bits 17–22 (RM0360 §6.4.6), which the F103's
    /// silicon-pinned AHBENR mask would discard — so the enable registers are
    /// stored unmasked, like an un-pinned F4 part.
    Stm32F0,
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
            "stm32g4" | "g4" => Ok(Self::Stm32G4),
            "stm32wb" | "wb" => Ok(Self::Stm32Wb),
            "stm32wba" | "wba" => Ok(Self::Stm32Wba),
            "stm32g0" | "g0" => Ok(Self::Stm32G0),
            "stm32f0" | "f0" => Ok(Self::Stm32F0),
            _ => Err(format!(
                "unsupported RCC register layout '{}'; supported: stm32f1, stm32f4, stm32v2, stm32h5, stm32h7, stm32l4, stm32l0, stm32g4, stm32wb, stm32wba, stm32g0, stm32f0",
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
///   U5    (CFGR1 @ 0x1C):     00 MSIS→2, 01 HSI16→10, 10 HSE→17, 11 PLL1→25.
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
            _ => {
                crate::census_reg!("rcc:F1Rcc", offset, "read");
                0
            }
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
            _ => {
                crate::census_reg!("rcc:F1Rcc", offset, "write");
            }
        }
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

// ── STM32F0 ─────────────────────────────────────────────────────────────────
// RM0360 Rev 5 §6.4 (STM32F030x4/x6/x8/xC, STM32F070x6/xB). Reference-manual
// derived; not silicon-verified (no F0 bench part).
#[derive(Debug, Default, serde::Serialize)]
pub struct F0Rcc {
    cr: u32,       // 0x00
    cfgr: u32,     // 0x04
    cir: u32,      // 0x08
    apb2rstr: u32, // 0x0C
    apb1rstr: u32, // 0x10
    ahbenr: u32,   // 0x14
    apb2enr: u32,  // 0x18
    apb1enr: u32,  // 0x1C
    bdcr: u32,     // 0x20 — LSEON bit0 → LSERDY bit1
    csr: u32,      // 0x24 — LSION bit0 → LSIRDY bit1
    ahbrstr: u32,  // 0x28
    cfgr2: u32,    // 0x2C — PREDIV[3:0]
    cfgr3: u32,    // 0x30 — USART1SW / I2C1SW / …
    cr2: u32,      // 0x34 — HSI14ON bit0 → HSI14RDY bit1, HSI48ON 16 → 17
}

/// RM0360 §6.4.14 RCC_CR2: each ON bit sets its RDY bit.
///   HSI14ON bit0 → HSI14RDY bit1, HSI48ON bit16 → HSI48RDY bit17.
fn f0_cr2_ready(mut cr2: u32) -> u32 {
    for &(on, rdy) in &[(0u32, 1u32), (16, 17)] {
        if cr2 & (1 << on) != 0 {
            cr2 |= 1 << rdy;
        } else {
            cr2 &= !(1 << rdy);
        }
    }
    cr2
}

/// Bit `on` of `value` decides bit `rdy` (the LSE/LSI ready pairs).
fn on_sets_ready(value: u32, on: u32, rdy: u32) -> u32 {
    if value & (1 << on) != 0 {
        value | (1 << rdy)
    } else {
        value & !(1 << rdy)
    }
}

impl F0Rcc {
    fn new() -> Self {
        Self {
            // RM0360 §6.4.1: reset 0x0000_XX83 — HSION|HSIRDY, HSITRIM = 0x10;
            // HSICAL (bits 15:8) is per-die factory trim, modelled as 0.
            cr: classic_cr_ready(0x0000_0083),
            // RM0360 §6.4.6: AHBENR reset 0x0000_0014 (SRAMEN | FLITFEN).
            ahbenr: 0x0000_0014,
            // RM0360 §6.4.10: CSR reset 0x0C00_0000 (NRSTF | PORRSTF).
            csr: 0x0C00_0000,
            // RM0360 §6.4.14: CR2 reset 0x0000_XX80 — HSI14TRIM = 0x10, off.
            cr2: 0x0000_0080,
            ..Default::default()
        }
    }
}

impl RccModel for F0Rcc {
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
            0x2C => self.cfgr2,
            0x30 => self.cfgr3,
            0x34 => self.cr2,
            _ => {
                crate::census_reg!("rcc:F0Rcc", offset, "read");
                0
            }
        }
    }
    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.cr = classic_cr_ready(value),
            // SW→SWS only follows once the requested source is ready in CR:
            // 00 HSI (bit1), 01 HSE (bit17), 10 PLL (bit25). 11 is HSI48 on
            // the F04x/F07x/F09x (ready in CR2, not CR) and reserved on the
            // F030/F070, so it never completes here.
            0x04 => {
                self.cfgr = cfgr_with_gated_sws(
                    value,
                    self.cr,
                    self.cfgr,
                    [Some(1), Some(17), Some(25), None],
                )
            }
            // CIR: only the interrupt-enable bits 13:8 hold state; the flags
            // (7:0) are read-only and the clear bits (23:16) write-only.
            0x08 => self.cir = value & 0x0000_3F00,
            0x0C => self.apb2rstr = value,
            0x10 => self.apb1rstr = value,
            0x14 => self.ahbenr = value,
            0x18 => self.apb2enr = value,
            0x1C => self.apb1enr = value,
            0x20 => self.bdcr = on_sets_ready(value, 0, 1),
            // CSR: LSION → LSIRDY; the reset flags (31:24) are plain storage.
            0x24 => self.csr = on_sets_ready(value, 0, 1),
            0x28 => self.ahbrstr = value,
            0x2C => self.cfgr2 = value & 0xF,
            0x30 => self.cfgr3 = value,
            0x34 => self.cr2 = f0_cr2_ready(value),
            _ => {
                crate::census_reg!("rcc:F0Rcc", offset, "write");
            }
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

/// RCC_CSR reset flags on the F4 (RM0368 §6.3.21): LPWRRSTF 31, WWDGRSTF 30,
/// IWDGRSTF 29, SFTRSTF 28, PORRSTF 27, PINRSTF 26, BORRSTF 25.
const F4_CSR_RESET_FLAGS: u32 = 0xFE00_0000;
/// RCC_CSR.RMVF: write 1 to clear every reset flag.
const F4_CSR_RMVF: u32 = 1 << 24;
const F4_CSR_BORRSTF: u32 = 1 << 25;
const F4_CSR_PINRSTF: u32 = 1 << 26;
const F4_CSR_PORRSTF: u32 = 1 << 27;

impl F4Rcc {
    /// A supply reset, as RM0368 §6.3.21 describes the flags: a POR/PDR sets
    /// PORRSTF and BORRSTF ("set by hardware when a POR/PDR or BOR reset
    /// occurs"), a brown-out sets BORRSTF alone, and either one drives the
    /// NRST pin low internally, so PINRSTF too. That is the CSR reset value,
    /// 0x0E00_0000, after a power-on.
    fn on_supply_reset(&mut self, cause: crate::power::SupplyResetCause) {
        let flags = match cause {
            crate::power::SupplyResetCause::PowerOn => {
                F4_CSR_PORRSTF | F4_CSR_BORRSTF | F4_CSR_PINRSTF
            }
            crate::power::SupplyResetCause::BrownOut => F4_CSR_BORRSTF | F4_CSR_PINRSTF,
        };
        self.csr |= flags;
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
            _ => {
                crate::census_reg!("rcc:F4Rcc", offset, "read");
                0
            }
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
            // CR ready rule. The reset flags (25:31) are set by hardware — the
            // supply supervisor sets POR/PIN/BOR, see `on_supply_reset` — and
            // cleared only by RMVF (bit 24, RM0368 §6.3.21), which reads 0.
            0x74 => {
                let flags = if value & F4_CSR_RMVF != 0 {
                    0
                } else {
                    self.csr & F4_CSR_RESET_FLAGS
                };
                let control = value & !(F4_CSR_RESET_FLAGS | F4_CSR_RMVF);
                let control = if control & 1 != 0 {
                    control | (1 << 1)
                } else {
                    control & !(1 << 1)
                };
                self.csr = flags | control;
            }
            _ => {
                crate::census_reg!("rcc:F4Rcc", offset, "write");
            }
        }
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

// ── STM32V2 (H5-style) ──────────────────────────────────────────────────────

/// Where a V2-family part puts its enable/reset register block.
///
/// The block placement is the main thing that moves between the WB and WBA
/// layouts; their clock tree (CR, ICSCR@0x04, CFGR@0x08, PLLCFGR@0x0C,
/// BDCR@0x90, CSR@0x94, CRRCR@0x98, EXTCFGR@0x108 for WB) is one
/// implementation. U5 (`Stm32V2`, RM0456) keeps this H5-style block but remaps
/// the clock tree around it (ICSCR1/2/3 @0x08/0x0C/0x10, CRRCR@0x14,
/// CFGR1@0x1C, AHB2ENR2@0x90, AHB3ENR@0x94, BDCR@0xF0, CSR@0xF4); those
/// deltas are selected by `V2Rcc::u5_cr_ready` in the register decode.
///
/// Keeping it a table rather than forking `V2Rcc` matters for more than
/// tidiness: [`Rcc::rcc_reg_offset`] — which resolves a peripheral's
/// `clock: { reg }` gate — reads the SAME table the register decode does, so
/// the gate and the decode cannot drift onto different offsets. That drift is
/// exactly the defect this type is being fixed for.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct V2EnrMap {
    ahbrstr: u64,
    apb1rstr: u64,
    apb2rstr: u64,
    ahbenr: u64,
    apb1enr: u64,
    apb2enr: u64,
}

impl V2EnrMap {
    /// H5 / WBA layout (RM0481 §11.8, RM0493 §11.8): AHB2ENR@0x8C,
    /// APB1ENR1(L)@0x9C, APB2ENR@0xA4. Verified against
    /// `tests/fixtures/real_world/stm32wba52.svd`.
    const H5_STYLE: Self = Self {
        ahbrstr: 0x6C,
        apb1rstr: 0x7C,
        apb2rstr: 0x84,
        ahbenr: 0x8C,
        apb1enr: 0x9C,
        apb2enr: 0xA4,
    };
    /// STM32WB layout (RM0434 §6.4): the L4-shaped block — AHB2RSTR@0x2C,
    /// APB1RSTR1@0x38, APB2RSTR@0x40, AHB2ENR@0x4C, APB1ENR1@0x58,
    /// APB2ENR@0x60. Verified against `tests/fixtures/real_world/stm32wb55.svd`
    /// and ST's CMSIS `stm32wb55xx.h`.
    const WB: Self = Self {
        ahbrstr: 0x2C,
        apb1rstr: 0x38,
        apb2rstr: 0x40,
        ahbenr: 0x4C,
        apb1enr: 0x58,
        apb2enr: 0x60,
    };
}

impl Default for V2EnrMap {
    fn default() -> Self {
        Self::H5_STYLE
    }
}

/// The full U5 (RM0456 §11.8) enable/reset block. Unlike [`V2EnrMap`] — whose
/// six slots are shared by WB/WBA — this is U5-only: eight reset registers at
/// 0x60..0x80 and eight enable registers at 0x88..0xA8, with 0x84 reserved.
/// It is the single source for both the U5 register decode and
/// [`Rcc::rcc_reg_offset`], so a U5 `clock:` gate cannot resolve to an offset
/// the decode does not honour.
///
/// Offsets from the vendored `configs/peripherals/stm32u575/rcc.yaml`
/// (generated from `tests/fixtures/real_world/stm32u575.svd`).
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct V2U5EnrMap {
    ahb1rstr: u64,  // 0x60
    ahb2rstr1: u64, // 0x64
    ahb2rstr2: u64, // 0x68
    ahb3rstr: u64,  // 0x6C
    apb1rstr1: u64, // 0x74
    apb1rstr2: u64, // 0x78
    apb2rstr: u64,  // 0x7C
    apb3rstr: u64,  // 0x80
    ahb1enr: u64,   // 0x88
    ahb2enr1: u64,  // 0x8C
    ahb2enr2: u64,  // 0x90
    ahb3enr: u64,   // 0x94
    apb1enr1: u64,  // 0x9C
    apb1enr2: u64,  // 0xA0
    apb2enr: u64,   // 0xA4
    apb3enr: u64,   // 0xA8
}

impl V2U5EnrMap {
    const U5: Self = Self {
        ahb1rstr: 0x60,
        ahb2rstr1: 0x64,
        ahb2rstr2: 0x68,
        ahb3rstr: 0x6C,
        apb1rstr1: 0x74,
        apb1rstr2: 0x78,
        apb2rstr: 0x7C,
        apb3rstr: 0x80,
        ahb1enr: 0x88,
        ahb2enr1: 0x8C,
        ahb2enr2: 0x90,
        ahb3enr: 0x94,
        apb1enr1: 0x9C,
        apb1enr2: 0xA0,
        apb2enr: 0xA4,
        apb3enr: 0xA8,
    };
}

#[derive(Debug, Default, serde::Serialize)]
pub struct V2Rcc {
    cr: u32,
    /// G4/WB MSI calibration at 0x04; U5 ICSCR1 at 0x08 (U5 has no register
    /// at 0x04). Plain storage on every V2 flavour.
    icscr: u32,
    /// U5 ICSCR2 @ 0x0C (RM0456 §11.8.4) — MSI trim, plain storage. On G4/WB
    /// 0x0C is RCC_PLLCFGR (`pllcfgr` below).
    icscr2: u32,
    /// U5 ICSCR3 @ 0x10 (RM0456 §11.8.5) — HSI calibration/trim, plain
    /// storage. 0x10 is reserved on G4/WB.
    icscr3: u32,
    cfgr: u32,    // 0x08 (G4/WB RM0440/RM0434: CR=0x00, ICSCR=0x04, CFGR=0x08)
    pllcfgr: u32, // 0x0C — G4/WB PLL config (HAL_RCC_GetSysClockFreq reads it)
    /// Enable/reset block placement — H5/WBA at 0x8C/0x9C/0xA4, WB at
    /// 0x4C/0x58/0x60. Skipped in the snapshot: it is fixed configuration,
    /// not simulated state, and emitting it would churn every V2 snapshot.
    #[serde(skip)]
    map: V2EnrMap,
    ahbenr: u32,   // AHB2ENR — `map.ahbenr`
    apb1enr: u32,  // APB1ENR1 / APB1LENR — `map.apb1enr`
    apb2enr: u32,  // APB2ENR — `map.apb2enr`
    ahbrstr: u32,  // `map.ahbrstr`
    apb1rstr: u32, // `map.apb1rstr`
    apb2rstr: u32, // `map.apb2rstr`
    bdcr: u32,     // 0x90 (WB/WBA, LSE only) / 0xF0 (U5: LSE + LSESYS + LSI pairs)
    csr: u32,      // 0x94 (WB/WBA: LSION→LSIRDY) / 0xF4 (U5: flags+ranges storage)
    /// HSI48 control: CRRCR@0x98 on WB/WBA (HSI48ON bit0 → HSI48RDY bit1);
    /// CRRCR@0x14 on U5 (RM0456 §11.8.6), same pair.
    crrcr: u32,
    bdcr1: u32, // 0xF0 — WBA backup domain: LSI/LSESYS/LSE2 enable→ready pairs
    /// AHB2ENR2 storage on U5 @ 0x90 (RM0456 §11.8.14) — 0x90 is BDCR on
    /// WB/WBA.
    ahb2enr2: u32,
    /// AHB3ENR storage on U5 @ 0x94 (RM0456 §11.8.15) — 0x94 is CSR on
    /// WB/WBA.
    ahb3enr: u32,
    /// U5-only enable registers (RM0456 §11.8.12-19), decoded through
    /// [`V2U5EnrMap`]: AHB1ENR@0x88, APB1ENR2@0xA0 and APB3ENR@0xA8. The
    /// shared `ahbenr`/`apb1enr`/`apb2enr` fields above carry AHB2ENR1/
    /// APB1ENR1/APB2ENR on U5 at the H5-style offsets.
    ahb1enr: u32,
    apb1enr2: u32,
    apb3enr: u32,
    /// U5-only reset registers (RM0456 §11.8.12-19): AHB1RSTR@0x60,
    /// AHB2RSTR1@0x64, AHB2RSTR2@0x68, AHB3RSTR@0x6C, APB1RSTR1@0x74,
    /// APB1RSTR2@0x78, APB3RSTR@0x80. APB2RSTR@0x7C reuses `apb2rstr`
    /// above. 0x84 is reserved on U5.
    ahb1rstr: u32,
    ahb2rstr1: u32,
    ahb2rstr2: u32,
    ahb3rstr: u32,
    apb1rstr1: u32,
    apb1rstr2: u32,
    apb3rstr: u32,
    /// U5 RCC_CFGR2 @ 0x20 (reset 0x6000: DPRE=÷16) / RCC_CFGR3 @ 0x24 —
    /// AHB/APB prescalers + per-domain clock-disable bits, plain storage.
    /// 0x20/0x24 are RCC_CFGR2/3 on WBA too, but WBA is out of scope here.
    cfgr2: u32,
    cfgr3: u32,
    /// U5 PLL2/PLL3 register blocks (RM0456 §11.8.8-11), plain storage with
    /// the SVD DIVR reset 0x0101_0280.
    pll2cfgr: u32, // 0x2C
    pll3cfgr: u32,  // 0x30
    pll2divr: u32,  // 0x3C
    pll2fracr: u32, // 0x40
    pll3divr: u32,  // 0x44
    pll3fracr: u32, // 0x48
    /// U5 RCC clock-interrupt block (RM0456 §11.8.20-22): CIER@0x50 enable
    /// storage, CIFR@0x54 flag storage, CICR@0x58 write-1-to-clear against
    /// CIFR (and reads 0 — write-only).
    cier: u32,
    cifr: u32,
    cfgr1: u32, // 0x1C — U5/WBA RCC_CFGR1 (SW→SWS); G4/WB use CFGR at 0x08
    /// WBA/U5 RCC_PLL1CFGR @ 0x28 (RM0493/RM0456). Plain storage; the
    /// `stm32wba` layout additionally synthesizes the read-only
    /// `PLL1RCLKPRERDY` status (see `synthesize_wba_rclk_pre_rdy`).
    pll1cfgr: u32, // 0x28
    /// WBA/U5 RCC_PLL1DIVR @ 0x34. Seeded to the vendored-SVD reset value
    /// 0x01010280 in `new()`.
    pll1divr: u32, // 0x34
    /// WBA/U5 RCC_PLL1FRACR @ 0x38.
    pll1fracr: u32, // 0x38
    /// WBA only (RM0493 §7.7.13): `PLL1RCLKPRE` bit20 is the
    /// divided/not-divided request and bit22 `PLL1RCLKPRERDY` is its
    /// read-only ready status. Zephyr's `clock_stm32_ll_wba.c` and the Cube
    /// HAL `HAL_RCC_ClockConfig` clear bit20 and spin on bit22, so the WBA
    /// layout reports `RDY = !PRE`; generic V2 (U5, RM0456) has no field at
    /// bits 19-31 and is plain storage. Fixed configuration, not simulated
    /// state — skipped in the snapshot for the same reason as `map`.
    #[serde(skip)]
    synthesize_wba_rclk_pre_rdy: bool,
    /// Whether this V2 instance follows the U5 (RM0456) CR ready layout —
    /// MSISRDY at bit2, MSIKRDY at bit5, HSI48 in CR bits 12/13 — rather than
    /// the classic G4/WB/WBA layout (MSIRDY bit1, HSI48 in CRRCR). Fixed
    /// configuration, not simulated state; skipped in the snapshot for the
    /// same reason as `map`.
    #[serde(skip)]
    u5_cr_ready: bool,
    /// STM32WB RCC_EXTCFGR @ 0x108 — shared/CPU2 AHB prescalers + ready flags
    /// (RM0434: SHDHPREF bit16, C2HPREF bit17). G4 has no EXTCFGR.
    extcfgr: u32,
}

fn pll1divr_reset() -> u32 {
    0x0101_0280
}

impl V2Rcc {
    fn new() -> Self {
        Self {
            cr: Self::ready_classic(1 << 0),
            pll1divr: pll1divr_reset(),
            ..Default::default()
        }
    }
    /// `stm32v2`/U5 (RM0456) instance — the only user of
    /// [`RccRegisterLayout::Stm32V2`]. The U5 CR layout is the H5 one, not the
    /// G4/WB classic layout the WB/WBA instances share: MSISRDY is bit2 (bit1
    /// is MSIKERON) and HSI48 lives in CR bits 12/13. The U5 clock tree is
    /// remapped on top of the H5-style enable block: ICSCR1/2/3 at
    /// 0x08/0x0C/0x10, CRRCR at 0x14, CFGR1 at 0x1C, CFGR2/3 at 0x20/0x24,
    /// the PLL2/PLL3 blocks at 0x2C/0x30/0x3C/0x40/0x44/0x48, CIER/CIFR/CICR
    /// at 0x50/0x54/0x58, AHB2ENR2 at 0x90, AHB3ENR at 0x94, BDCR at 0xF0
    /// and CSR at 0xF4. Seed the SVD resets from the vendored `rcc.yaml`:
    /// CR 0x35 (MSISON|MSISRDY|MSIKON|MSIKRDY), the three ICSCR factory
    /// calibration values (0x44000000 / 0x084210 / 0x00100000), AHB1ENR
    /// 0xD0200100, AHB2ENR1 0xC0000000, AHB2ENR2/AHB3ENR 0x80000000,
    /// CFGR2 0x6000 (DPRE=÷16), PLL2DIVR/PLL3DIVR 0x01010280 and CSR
    /// 0x0C004400 (MSI ranges + PINRSTF|BORRSTF).
    fn new_u5() -> Self {
        Self {
            cr: Self::ready_u5(0x35),
            icscr: 0x4400_0000,
            icscr2: 0x0008_4210,
            icscr3: 0x0010_0000,
            u5_cr_ready: true,
            ahb1enr: 0xD020_0100,
            ahbenr: 0xC000_0000,
            ahb2enr2: 0x8000_0000,
            ahb3enr: 0x8000_0000,
            cfgr2: 0x0000_6000,
            pll2divr: pll1divr_reset(),
            pll3divr: pll1divr_reset(),
            csr: 0x0C00_4400,
            ..Self::new()
        }
    }
    /// Same model, STM32WB enable/reset placement (RM0434 §6.4).
    fn new_wb() -> Self {
        Self {
            map: V2EnrMap::WB,
            ..Self::new()
        }
    }
    /// Same model, WBA (RM0493) PLL1CFGR semantics: bit22 `PLL1RCLKPRERDY`
    /// is a read-only status tracking the inverse of bit20 `PLL1RCLKPRE`
    /// (see `synthesize_wba_rclk_pre_rdy`).
    fn new_wba() -> Self {
        Self {
            synthesize_wba_rclk_pre_rdy: true,
            ..Self::new()
        }
    }
    /// WB/WBA classic CR ready rule: the classic HSI(0)/HSE(16)/PLL(24) bits
    /// plus HSI16 at bit8→bit10 (these families gate the kernel clock on
    /// HSI16RDY, e.g. WBA's stm32_clock_control_init).
    fn ready_classic(cr: u32) -> u32 {
        let mut cr = classic_cr_ready(cr);
        if cr & (1 << 8) != 0 {
            cr |= 1 << 10;
        } else {
            cr &= !(1 << 10);
        }
        cr
    }
    /// U5 (RM0456) CR ready-flag rule, at the H5 bit positions:
    /// MSISON(0)→MSISRDY(2), MSIKON(4)→MSIKRDY(5), HSION(8)→HSIRDY(10),
    /// HSI48ON(12)→HSI48RDY(13), HSEON(16)→HSERDY(17), PLL1ON(24)→PLL1RDY(25),
    /// PLL2ON(26)→PLL2RDY(27), PLL3ON(28)→PLL3RDY(29). SHSI (14/15) is not
    /// modelled — no shipped firmware starts it. RDY is a pure status: it
    /// follows ON on every write, exactly as the per-family parent/sibling
    /// models latch their CR ready bits.
    fn ready_u5(mut cr: u32) -> u32 {
        for &(on, rdy) in &[
            (0u32, 2u32),
            (4, 5),
            (8, 10),
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
        cr
    }
}

impl RccModel for V2Rcc {
    fn read_reg(&self, offset: u64) -> u32 {
        // Family-placed enable/reset block first. On U5 this is the full
        // 16-register block from `V2U5EnrMap`; on WB/WBA the six shared slots
        // from `V2EnrMap`. The two are separate tables so a U5 offset (0x60,
        // 0x74, 0x88, 0xA0, 0xA8 …) can never resolve through the H5/WBA
        // slots — the defect that put U5's reset block on WBA's offsets.
        if self.u5_cr_ready {
            let m = V2U5EnrMap::U5;
            if offset == m.ahb1rstr {
                return self.ahb1rstr;
            } else if offset == m.ahb2rstr1 {
                return self.ahb2rstr1;
            } else if offset == m.ahb2rstr2 {
                return self.ahb2rstr2;
            } else if offset == m.ahb3rstr {
                return self.ahb3rstr;
            } else if offset == m.apb1rstr1 {
                return self.apb1rstr1;
            } else if offset == m.apb1rstr2 {
                return self.apb1rstr2;
            } else if offset == m.apb2rstr {
                return self.apb2rstr;
            } else if offset == m.apb3rstr {
                return self.apb3rstr;
            } else if offset == m.ahb1enr {
                return self.ahb1enr;
            } else if offset == m.ahb2enr1 {
                return self.ahbenr;
            } else if offset == m.ahb2enr2 {
                return self.ahb2enr2;
            } else if offset == m.ahb3enr {
                return self.ahb3enr;
            } else if offset == m.apb1enr1 {
                return self.apb1enr;
            } else if offset == m.apb1enr2 {
                return self.apb1enr2;
            } else if offset == m.apb2enr {
                return self.apb2enr;
            } else if offset == m.apb3enr {
                return self.apb3enr;
            }
        } else {
            let m = self.map;
            if offset == m.ahbrstr {
                return self.ahbrstr;
            } else if offset == m.apb1rstr {
                return self.apb1rstr;
            } else if offset == m.apb2rstr {
                return self.apb2rstr;
            } else if offset == m.ahbenr {
                return self.ahbenr;
            } else if offset == m.apb1enr {
                return self.apb1enr;
            } else if offset == m.apb2enr {
                return self.apb2enr;
            }
        }
        match offset {
            0x00 => self.cr,
            // 0x04 is G4/WB RCC_ICSCR; U5 (RM0456) has no register there —
            // its ICSCR1 is at 0x08.
            0x04 if !self.u5_cr_ready => self.icscr,
            0x08 if self.u5_cr_ready => self.icscr, // U5 ICSCR1
            0x08 => self.cfgr,                      // G4/WB RCC_CFGR
            0x0C if self.u5_cr_ready => self.icscr2, // U5 ICSCR2
            0x0C => self.pllcfgr,                   // G4/WB RCC_PLLCFGR
            0x10 if self.u5_cr_ready => self.icscr3, // U5 ICSCR3
            0x14 if self.u5_cr_ready => self.crrcr, // U5 CRRCR
            0x1C => self.cfgr1,
            // U5 RCC_CFGR2 @ 0x20 (AHB/APB prescalers + domain clock-disable)
            // and CFGR3 @ 0x24 (PPRE3/AHB3DIS/APB3DIS) — plain storage.
            0x20 if self.u5_cr_ready => self.cfgr2,
            0x24 if self.u5_cr_ready => self.cfgr3,
            // U5/WBA PLL1 block: PLL1DIVR/PLL1FRACR are ordinary storage.
            // PLL1CFGR is storage too, except on WBA where bit22
            // `PLL1RCLKPRERDY` is a read-only status that hardware sets once
            // bit20 `PLL1RCLKPRE` goes divided→not-divided. The Cube HAL and
            // Zephyr both clear bit20 then poll bit22 (`RDY = !PRE`); U5 has
            // no field at bit22 at all (RM0456 §7.7.13: bits 19-31 reserved),
            // so only the `stm32wba` layout synthesizes the status.
            0x28 => {
                if self.synthesize_wba_rclk_pre_rdy {
                    if self.pll1cfgr & (1 << 20) == 0 {
                        self.pll1cfgr | (1 << 22)
                    } else {
                        self.pll1cfgr & !(1 << 22)
                    }
                } else {
                    self.pll1cfgr
                }
            }
            0x2C if self.u5_cr_ready => self.pll2cfgr,
            0x30 if self.u5_cr_ready => self.pll3cfgr,
            0x34 => self.pll1divr,
            0x38 => self.pll1fracr,
            0x3C if self.u5_cr_ready => self.pll2divr,
            0x40 if self.u5_cr_ready => self.pll2fracr,
            0x44 if self.u5_cr_ready => self.pll3divr,
            0x48 if self.u5_cr_ready => self.pll3fracr,
            // U5 clock-interrupt block (RM0456 §11.8.20-22): CIER enables,
            // CIFR flags, CICR write-1-to-clear (write-only, reads 0).
            0x50 if self.u5_cr_ready => self.cier,
            0x54 if self.u5_cr_ready => self.cifr,
            0x58 if self.u5_cr_ready => 0,
            // 0x90/0x94/0x98 are the U5/G4/WB split: U5 has AHB2ENR2 at 0x90,
            // AHB3ENR at 0x94 and no register at 0x98; WB/WBA have the backup
            // domain at 0x90/0x94 (BDCR/CSR) and CRRCR at 0x98. The U5 arms
            // live in the `V2U5EnrMap` block above; only the WB/WBA arms
            // remain here.
            0x90 => self.bdcr,
            0x94 => self.csr,
            0x98 if !self.u5_cr_ready => self.crrcr,
            0xF0 if self.u5_cr_ready => self.bdcr,
            0xF0 => self.bdcr1,
            0xF4 if self.u5_cr_ready => self.csr,
            // STM32WB EXTCFGR — dual-core AHB prescalers (CPU2 / shared domain).
            0x108 => self.extcfgr,
            _ => {
                crate::census_reg!("rcc:V2Rcc", offset, "read");
                0
            }
        }
    }
    fn write_reg(&mut self, offset: u64, value: u32) {
        // Family-placed enable/reset block — see `read_reg`.
        if self.u5_cr_ready {
            let m = V2U5EnrMap::U5;
            if offset == m.ahb1rstr {
                self.ahb1rstr = value;
                return;
            } else if offset == m.ahb2rstr1 {
                self.ahb2rstr1 = value;
                return;
            } else if offset == m.ahb2rstr2 {
                self.ahb2rstr2 = value;
                return;
            } else if offset == m.ahb3rstr {
                self.ahb3rstr = value;
                return;
            } else if offset == m.apb1rstr1 {
                self.apb1rstr1 = value;
                return;
            } else if offset == m.apb1rstr2 {
                self.apb1rstr2 = value;
                return;
            } else if offset == m.apb2rstr {
                self.apb2rstr = value;
                return;
            } else if offset == m.apb3rstr {
                self.apb3rstr = value;
                return;
            } else if offset == m.ahb1enr {
                self.ahb1enr = value;
                return;
            } else if offset == m.ahb2enr1 {
                self.ahbenr = value;
                return;
            } else if offset == m.ahb2enr2 {
                self.ahb2enr2 = value;
                return;
            } else if offset == m.ahb3enr {
                self.ahb3enr = value;
                return;
            } else if offset == m.apb1enr1 {
                self.apb1enr = value;
                return;
            } else if offset == m.apb1enr2 {
                self.apb1enr2 = value;
                return;
            } else if offset == m.apb2enr {
                self.apb2enr = value;
                return;
            } else if offset == m.apb3enr {
                self.apb3enr = value;
                return;
            }
        } else {
            let m = self.map;
            if offset == m.ahbrstr {
                self.ahbrstr = value;
                return;
            } else if offset == m.apb1rstr {
                self.apb1rstr = value;
                return;
            } else if offset == m.apb2rstr {
                self.apb2rstr = value;
                return;
            } else if offset == m.ahbenr {
                self.ahbenr = value;
                return;
            } else if offset == m.apb1enr {
                self.apb1enr = value;
                return;
            } else if offset == m.apb2enr {
                self.apb2enr = value;
                return;
            }
        }
        match offset {
            0x00 => {
                self.cr = if self.u5_cr_ready {
                    Self::ready_u5(value)
                } else {
                    Self::ready_classic(value)
                }
            }
            // 0x04/0x08/0x0C/0x10 are the U5/G4/WB split. U5 (RM0456) has no
            // register at 0x04; its ICSCR1/2/3 are at 0x08/0x0C/0x10 and are
            // plain storage. G4/WB have ICSCR at 0x04, CFGR at 0x08 and
            // PLLCFGR at 0x0C (0x10 reserved).
            0x04 if !self.u5_cr_ready => self.icscr = value,
            0x08 if self.u5_cr_ready => self.icscr = value, // ICSCR1
            // G4/WB RCC_CFGR (0x08): SW→SWS follows only once the requested
            // source is ready in CR — 00 MSI (bit1), 01 HSI16 (bit10),
            // 10 HSE (bit17), 11 PLL (bit25).
            //
            // STM32WB (RM0434) also exposes live prescaler-applied flags that
            // HAL_RCC_ClockConfig polls with a 2 ms timeout:
            //   bit16 HPREF, bit17 PPRE1F, bit18 PPRE2F.
            // Silicon sets them once the new divider is in force; we set them
            // immediately on any CFGR write so the poll succeeds.
            0x08 => {
                let mut cfgr = cfgr_with_gated_sws(
                    value,
                    self.cr,
                    self.cfgr,
                    [Some(1), Some(10), Some(17), Some(25)],
                );
                // Flags are read-only status in silicon; firmware writes never
                // clear them. Always assert applied.
                cfgr |= (1 << 16) | (1 << 17) | (1 << 18);
                self.cfgr = cfgr;
            }
            0x0C if self.u5_cr_ready => self.icscr2 = value, // ICSCR2
            0x0C => self.pllcfgr = value,
            0x10 if self.u5_cr_ready => self.icscr3 = value, // ICSCR3
            // WB EXTCFGR @ 0x108: SHDHPRE[3:0], C2HPRE[7:4]; SHDHPREF (16) and
            // C2HPREF (17) go high once the write is accepted.
            0x108 => {
                self.extcfgr = (value & 0x0000_00FF) | (1 << 16) | (1 << 17);
            }
            // U5/WBA RCC_CFGR1 (0x1C): SW[1:0]→SWS[3:2], gated on the source's
            // CR ready bit. U5 (RM0456): 00 MSIS (bit2), 01 HSI16 (bit10),
            // 10 HSE (bit17), 11 PLL1 (bit25). WBA (RM0493): 00 HSI16 (bit10),
            // 01 reserved, 10 HSE (bit17), 11 PLL1R (bit25).
            0x1C => {
                let rdy_bits = if self.u5_cr_ready {
                    [Some(2), Some(10), Some(17), Some(25)]
                } else {
                    [Some(10), None, Some(17), Some(25)]
                };
                self.cfgr1 = cfgr_with_gated_sws(value, self.cr, self.cfgr1, rdy_bits)
            }
            0x14 if self.u5_cr_ready => {
                self.crrcr = if value & 1 != 0 {
                    value | (1 << 1)
                } else {
                    value & !(1 << 1)
                };
            }
            // PLL1 configuration is frozen while PLL1ON is set (RM0456 §11.8.9).
            0x28 if self.cr & (1 << 24) == 0 => self.pll1cfgr = value,
            0x34 if self.cr & (1 << 24) == 0 => self.pll1divr = value,
            0x38 if self.cr & (1 << 24) == 0 => self.pll1fracr = value,
            0x28 | 0x34 | 0x38 => {}
            // U5 RCC_CFGR2/3 and the PLL2/PLL3 CFGR/DIVR/FRACR blocks — plain
            // storage (RM0456 §11.8.7-11).
            0x20 if self.u5_cr_ready => self.cfgr2 = value,
            0x24 if self.u5_cr_ready => self.cfgr3 = value,
            0x2C if self.u5_cr_ready => self.pll2cfgr = value,
            0x30 if self.u5_cr_ready => self.pll3cfgr = value,
            0x3C if self.u5_cr_ready => self.pll2divr = value,
            0x40 if self.u5_cr_ready => self.pll2fracr = value,
            0x44 if self.u5_cr_ready => self.pll3divr = value,
            0x48 if self.u5_cr_ready => self.pll3fracr = value,
            // U5 clock-interrupt block: CIER/CIFR storage, CICR W1C against
            // CIFR and write-only (reads 0). Mirrors the H7 model.
            0x50 if self.u5_cr_ready => self.cier = value,
            0x54 if self.u5_cr_ready => self.cifr = value,
            0x58 if self.u5_cr_ready => {
                self.cifr &= !value;
            }
            // U5 AHB2ENR2/AHB3ENR are decoded by the `V2U5EnrMap` block above.
            // WB/WBA keep the backup-domain handshakes: BDCR 0x90 LSEON bit0 →
            // LSERDY bit1, CSR 0x94 LSION bit0 → LSIRDY bit1, CRRCR 0x98
            // HSI48ON bit0 → HSI48RDY bit1. U5 moves the whole backup domain to
            // BDCR@0xF0 (LSE/LSESYS/LSI pairs) and keeps only flags/ranges in
            // CSR@0xF4; it has no register at 0x98.
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
            // CRRCR: HSI48ON (bit0) → HSI48RDY (bit1).
            0x98 if !self.u5_cr_ready => {
                self.crrcr = if value & 1 != 0 {
                    value | (1 << 1)
                } else {
                    value & !(1 << 1)
                };
            }
            // U5 BDCR @ 0xF0 (RM0456 §11.8.40): the backup domain keeps all
            // three enable→ready handshakes — LSEON(0)→LSERDY(1),
            // LSESYSEN(7)→LSESYSRDY(11) and LSION(26)→LSIRDY(27). CubeU5's
            // `HAL_RCC_OscConfig` polls all three (stm32u5xx_hal_rcc.c), so a
            // partial LSE-only ack hangs the boot on bit11/bit27.
            0xF0 if self.u5_cr_ready => {
                let mut v = value;
                for &(on, rdy) in &[(0u32, 1u32), (7, 11), (26, 27)] {
                    if v & (1 << on) != 0 {
                        v |= 1 << rdy;
                    } else {
                        v &= !(1 << rdy);
                    }
                }
                self.bdcr = v;
            }
            // U5 CSR @ 0xF4 (RM0456 §11.8.41): reset flags (25-31) plus the
            // MSIS/MSIK range fields. LSI lives in BDCR on U5, so there is no
            // LSION→LSIRDY pair here. RMVF (bit 23) is write-1-to-clear for
            // OBLRSTF..LPWRRSTF and self-clearing (reads 0).
            0xF4 if self.u5_cr_ready => {
                let mut v = value & !(1 << 23);
                if value & (1 << 23) != 0 {
                    v &= !(0xFE00_0000);
                }
                self.csr = v;
            }
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
            _ => {
                crate::census_reg!("rcc:V2Rcc", offset, "write");
            }
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
    pllcfgr: [u32; 3], // 0x28 / 0x2C / 0x30 — PLL1CFGR / PLL2CFGR / PLL3CFGR
    // PLL divider block (RM0481 / stm32h563xx.h). HAL_RCC_GetSysClockFreq
    // reads PLL1DIVR for N and P and PLL1FRACR for the fractional part.
    // HAL_RCC_DeInit restores each DIVR to 0x01010280 and clears each FRACR.
    pll1divr: u32,  // 0x34
    pll1fracr: u32, // 0x38
    pll2divr: u32,  // 0x3C
    pll2fracr: u32, // 0x40
    pll3divr: u32,  // 0x44
    pll3fracr: u32, // 0x48
    ahb1rstr: u32,  // 0x60
    ahb2rstr: u32,  // 0x64
    apb1lrstr: u32, // 0x74
    apb1hrstr: u32, // 0x78
    apb2rstr: u32,  // 0x7C
    apb3rstr: u32,  // 0x80
    ahb1enr: u32,   // 0x88 — reset 0xD0000100
    ahb2enr: u32,   // 0x8C — reset 0xC0000000 (SRAM2EN|SRAM3EN)
    apb1lenr: u32,  // 0x9C
    apb1henr: u32,  // 0xA0
    apb2enr: u32,   // 0xA4
    apb3enr: u32,   // 0xA8
    bdcr: u32,      // 0xF0
    rsr: u32,       // 0xF4 — reset 0x0C000000 (PINRST|BORRST)
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
            pll1divr: 0x0101_0280,
            pll1fracr: 0,
            pll2divr: 0x0101_0280,
            pll2fracr: 0,
            pll3divr: 0x0101_0280,
            pll3fracr: 0,
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
            0x34 => self.pll1divr,
            0x38 => self.pll1fracr,
            0x3C => self.pll2divr,
            0x40 => self.pll2fracr,
            0x44 => self.pll3divr,
            0x48 => self.pll3fracr,
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
            _ => {
                crate::census_reg!("rcc:H5Rcc", offset, "read");
                0
            }
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
            0x34 => self.pll1divr = value,
            0x38 => self.pll1fracr = value,
            0x3C => self.pll2divr = value,
            0x40 => self.pll2fracr = value,
            0x44 => self.pll3divr = value,
            0x48 => self.pll3fracr = value,
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
            _ => {
                crate::census_reg!("rcc:H5Rcc", offset, "write");
            }
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
            _ => {
                crate::census_reg!("rcc:H7Rcc", offset, "read");
                0
            }
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
            _ => {
                crate::census_reg!("rcc:H7Rcc", offset, "write");
            }
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
            _ => {
                crate::census_reg!("rcc:L4Rcc", offset, "read");
                0
            }
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
            _ => {
                crate::census_reg!("rcc:L4Rcc", offset, "write");
            }
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
            _ => {
                crate::census_reg!("rcc:L0Rcc", offset, "read");
                0
            }
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
            _ => {
                crate::census_reg!("rcc:L0Rcc", offset, "write");
            }
        }
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

// ── STM32G4 ─────────────────────────────────────────────────────────────────
// RM0440 register map. Shares the modern-V2 clock-tree IP (CR/CFGR/PLLCFGR with
// the classic + HSI16 ready rules, BDCR/CSR/CRRCR handshakes), but the
// enable/reset registers sit at the RM0440 offsets — APB1ENR1@0x58,
// APB1ENR2@0x5C, AHB1ENR@0x48, AHB2ENR@0x4C, APB2ENR@0x60, reset regs at
// 0x28/0x2C/0x38/0x3C/0x40. On the H5-style V2 layout those same enables live at
// 0x9C/0xA4, so I2C1EN (APB1ENR1 bit 21) written by `__HAL_RCC_I2C1_CLK_ENABLE`
// would be dropped and the clock-gate check would read the wrong register. A
// dedicated struct keeps G4 and the H5-style V2 families apart. Offsets/bit
// positions verified against the vendored CMSIS `stm32g474xx.h`.
#[derive(Debug, Default, serde::Serialize)]
pub struct G4Rcc {
    cr: u32,        // 0x00
    icscr: u32,     // 0x04
    cfgr: u32,      // 0x08
    pllcfgr: u32,   // 0x0C
    ahb1rstr: u32,  // 0x28
    ahb2rstr: u32,  // 0x2C
    apb1rstr: u32,  // APB1RSTR1 0x38
    apb1rstr2: u32, // APB1RSTR2 0x3C
    apb2rstr: u32,  // 0x40
    ahb1enr: u32,   // 0x48
    ahb2enr: u32,   // AHB2ENR 0x4C (GPIO ports, ADC12)
    apb1enr: u32,   // APB1ENR1 0x58 (TIM2, RTCAPB, I2C1)
    apb1enr2: u32,  // APB1ENR2 0x5C
    apb2enr: u32,   // 0x60 (TIM1, SPI1)
    bdcr: u32,      // 0x90 — LSEON bit0 → LSERDY bit1
    csr: u32,       // 0x94 — LSION bit0 → LSIRDY bit1
    crrcr: u32,     // 0x98 — HSI48ON bit0 → HSI48RDY bit1
}

impl G4Rcc {
    fn new() -> Self {
        Self {
            cr: Self::ready(1 << 0),
            ..Default::default()
        }
    }
    /// G4 CR ready rule (matches the V2 model that boots this family): the
    /// classic HSION(0)/HSERDY(16)/PLLRDY(24) bits plus HSI16 at bit8→bit10 —
    /// the Arduino G4 core kernel clock gates on HSI16RDY.
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

impl RccModel for G4Rcc {
    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.icscr,
            0x08 => self.cfgr,
            0x0C => self.pllcfgr,
            0x28 => self.ahb1rstr,
            0x2C => self.ahb2rstr,
            0x38 => self.apb1rstr,
            0x3C => self.apb1rstr2,
            0x40 => self.apb2rstr,
            0x48 => self.ahb1enr,
            0x4C => self.ahb2enr,
            0x58 => self.apb1enr,
            0x5C => self.apb1enr2,
            0x60 => self.apb2enr,
            0x90 => self.bdcr,
            0x94 => self.csr,
            0x98 => self.crrcr,
            _ => {
                crate::census_reg!("rcc:G4Rcc", offset, "read");
                0
            }
        }
    }
    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.cr = Self::ready(value),
            0x04 => self.icscr = value,
            // RCC_CFGR (0x08): SW[1:0]→SWS[3:2] follows only once the requested
            // source is ready in CR — 01 HSI16 (bit10), 10 HSE (bit17), 11 PLL
            // (bit25). Slot 00 (bit1) is the classic-HSI slot the HAL never
            // selects on G4 but is kept for parity with the V2 model.
            0x08 => {
                self.cfgr = cfgr_with_gated_sws(
                    value,
                    self.cr,
                    self.cfgr,
                    [Some(1), Some(10), Some(17), Some(25)],
                );
            }
            0x0C => {
                self.pllcfgr = value;
                self.cr = Self::ready(self.cr); // PLLSRC change can re-gate PLLRDY
            }
            0x28 => self.ahb1rstr = value,
            0x2C => self.ahb2rstr = value,
            0x38 => self.apb1rstr = value,
            0x3C => self.apb1rstr2 = value,
            0x40 => self.apb2rstr = value,
            0x48 => self.ahb1enr = value,
            0x4C => self.ahb2enr = value,
            0x58 => self.apb1enr = value,
            0x5C => self.apb1enr2 = value,
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
            // CRRCR: HSI48ON (bit0) → HSI48RDY (bit1).
            0x98 => {
                self.crrcr = if value & 1 != 0 {
                    value | (1 << 1)
                } else {
                    value & !(1 << 1)
                };
            }
            _ => {
                crate::census_reg!("rcc:G4Rcc", offset, "write");
            }
        }
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

// ── STM32G0 ─────────────────────────────────────────────────────────────────
// RM0444 §5 register map, cross-checked against ST's CMSIS `stm32g071xx.h`
// RCC_TypeDef. The G0 register file is its own shape: CR@0x00, ICSCR@0x04,
// CFGR@0x08, PLLCFGR@0x0C, the clock-interrupt block CIER/CIFR/CICR at
// 0x18/0x1C/0x20, then the reset/enable block IOPRSTR@0x24 … IOPENR@0x34,
// AHBENR@0x38, APBENR1@0x3C, APBENR2@0x40, the sleep-enable mirrors at
// 0x44–0x50, CCIPR@0x54, BDCR@0x5C and CSR@0x60.
//
// CR ready flags sit at HSI16-style bit positions: HSION bit8 → HSIRDY bit10,
// HSEON bit16 → HSERDY bit17, PLLON bit24 → PLLRDY bit25 (RM0444 §5.4.1).
// G071 has no HSI48 and therefore no CRRCR register — unlike the L0 (CRRCR
// @0x08) whose 2-bit CFGR this map must not share.
//
// CFGR.SW and CFGR.SWS are 3-bit fields (SW[2:0] bits 0–2, SWS[2:0] bits
// 3–5). SW encodings per `stm32g071xx.h` RCC_CFGR_SW_*: 000 HSISYS, 001 HSE,
// 010 PLLRCLK, 011 LSI, 100 LSE; 101–111 reserved. The switch completes only
// when the requested source is up (the same source-ready gate the other
// families apply), so firmware that asks for a clock it never enabled sees
// SWS hold.
#[derive(Debug, Default, serde::Serialize)]
pub struct G0Rcc {
    cr: u32,        // 0x00 — HSION bit8 → HSIRDY bit10
    icscr: u32,     // 0x04 — HSI16 calibration, plain storage
    cfgr: u32,      // 0x08 — SW[2:0] → SWS[5:3]
    pllcfgr: u32,   // 0x0C — plain storage
    cier: u32,      // 0x18
    cifr: u32,      // 0x1C
    cicr: u32,      // 0x20
    ioprstr: u32,   // 0x24
    ahbrstr: u32,   // 0x28
    apbrstr1: u32,  // 0x2C
    apbrstr2: u32,  // 0x30
    iopenr: u32,    // 0x34 — GPIO port clock enable (GPIOAEN bit0 … GPIOFEN bit5)
    ahbenr: u32,    // 0x38 — DMA1EN bit0, FLASHEN bit8, CRCEN bit12
    apbenr1: u32,   // 0x3C — USART2EN bit17, I2C1EN bit21, TIM2EN bit0
    apbenr2: u32,   // 0x40 — TIM1EN bit11, SPI1EN bit12, USART1EN bit14
    iopsmenr: u32,  // 0x44
    ahbsmenr: u32,  // 0x48
    apbsmenr1: u32, // 0x4C
    apbsmenr2: u32, // 0x50
    ccipr: u32,     // 0x54
    bdcr: u32,      // 0x5C — LSEON bit0 → LSERDY bit1
    csr: u32,       // 0x60 — LSION bit0 → LSIRDY bit1, reset flags 31:24
}

impl G0Rcc {
    fn new() -> Self {
        // RM0444 §5.4.1: RCC_CR power-on reset value 0x0000_0500
        // (HSION bit8 | HSIRDY bit10). HSIRDY is a status the model tracks.
        let mut s = Self {
            cr: 0x0000_0500,
            ..Default::default()
        };
        s.cr = Self::ready(s.cr);
        s
    }

    /// G0 CR ready rule: HSION(8)→HSIRDY(10), HSEON(16)→HSERDY(17),
    /// PLLON(24)→PLLRDY(25). HSEBYP is not gated on here — the L4 model's
    /// bypass gate exists because CubeL4 polls it; the G0 smoke path and the
    /// HAL bring-up only need the ON→RDY pair to latch.
    fn ready(mut cr: u32) -> u32 {
        for &(on, rdy) in &[(8u32, 10u32), (16, 17), (24, 25)] {
            if cr & (1 << on) != 0 {
                cr |= 1 << rdy;
            } else {
                cr &= !(1 << rdy);
            }
        }
        cr
    }
}

impl RccModel for G0Rcc {
    fn read_reg(&self, offset: u64) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.icscr,
            0x08 => self.cfgr,
            0x0C => self.pllcfgr,
            0x18 => self.cier,
            0x1C => self.cifr,
            0x20 => self.cicr,
            0x24 => self.ioprstr,
            0x28 => self.ahbrstr,
            0x2C => self.apbrstr1,
            0x30 => self.apbrstr2,
            0x34 => self.iopenr,
            0x38 => self.ahbenr,
            0x3C => self.apbenr1,
            0x40 => self.apbenr2,
            0x44 => self.iopsmenr,
            0x48 => self.ahbsmenr,
            0x4C => self.apbsmenr1,
            0x50 => self.apbsmenr2,
            0x54 => self.ccipr,
            0x5C => self.bdcr,
            0x60 => self.csr,
            _ => {
                crate::census_reg!("rcc:G0Rcc", offset, "read");
                0
            }
        }
    }

    fn write_reg(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => self.cr = Self::ready(value),
            0x04 => self.icscr = value,
            // RCC_CFGR (RM0444 §5.4.3): SW[2:0] → SWS[5:3] follows only once
            // the requested source is ready. Source → CR ready bit:
            // 000 HSISYS→bit10, 001 HSE→bit17, 010 PLLRCLK→bit25;
            // 011 LSI→CSR bit1, 100 LSE→BDCR bit1. 101-111 reserved.
            0x08 => {
                let sw = value & 0x7;
                let ready = match sw {
                    0 => self.cr & (1 << 10) != 0,
                    1 => self.cr & (1 << 17) != 0,
                    2 => self.cr & (1 << 25) != 0,
                    3 => self.csr & (1 << 1) != 0,
                    4 => self.bdcr & (1 << 1) != 0,
                    _ => false,
                };
                let sws = if ready {
                    sw << 3
                } else {
                    self.cfgr & (0x7 << 3)
                };
                self.cfgr = (value & !(0x7 << 3)) | sws;
            }
            0x0C => self.pllcfgr = value,
            0x18 => self.cier = value,
            0x1C => self.cifr = value,
            // CICR is write-1-to-clear against CIFR (RM0444 §5.4.7).
            0x20 => {
                self.cifr &= !value;
                self.cicr = 0;
            }
            0x24 => self.ioprstr = value,
            0x28 => self.ahbrstr = value,
            0x2C => self.apbrstr1 = value,
            0x30 => self.apbrstr2 = value,
            0x34 => self.iopenr = value,
            0x38 => self.ahbenr = value,
            0x3C => self.apbenr1 = value,
            0x40 => self.apbenr2 = value,
            0x44 => self.iopsmenr = value,
            0x48 => self.ahbsmenr = value,
            0x4C => self.apbsmenr1 = value,
            0x50 => self.apbsmenr2 = value,
            0x54 => self.ccipr = value,
            // BDCR: LSEON (bit0) → LSERDY (bit1); rest is RTC/backup storage.
            0x5C => {
                self.bdcr = if value & 1 != 0 {
                    value | (1 << 1)
                } else {
                    value & !(1 << 1)
                };
            }
            // CSR: LSION (bit0) → LSIRDY (bit1); reset flags (31:23) storage.
            0x60 => {
                self.csr = if value & 1 != 0 {
                    value | (1 << 1)
                } else {
                    value & !(1 << 1)
                };
            }
            _ => {
                crate::census_reg!("rcc:G0Rcc", offset, "write");
            }
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
    Stm32G4(G4Rcc),
    Stm32G0(G0Rcc),
    Stm32F0(F0Rcc),
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
            RccRegisterLayout::Stm32V2 => Self::Stm32V2(V2Rcc::new_u5()),
            RccRegisterLayout::Stm32H5 => Self::Stm32H5(H5Rcc::new()),
            RccRegisterLayout::Stm32H7 => Self::Stm32H7(H7Rcc::new()),
            RccRegisterLayout::Stm32L4 => Self::Stm32L4(L4Rcc::new()),
            RccRegisterLayout::Stm32L0 => Self::Stm32L0(L0Rcc::new()),
            RccRegisterLayout::Stm32G4 => Self::Stm32G4(G4Rcc::new()),
            RccRegisterLayout::Stm32G0 => Self::Stm32G0(G0Rcc::new()),
            RccRegisterLayout::Stm32F0 => Self::Stm32F0(F0Rcc::new()),
            // Same V2 model, RM0434 enable/reset placement.
            RccRegisterLayout::Stm32Wb => Self::Stm32V2(V2Rcc::new_wb()),
            // Same V2 model, RM0493 PLL1RCLKPRE→PLL1RCLKPRERDY status.
            RccRegisterLayout::Stm32Wba => Self::Stm32V2(V2Rcc::new_wba()),
        }
    }

    /// Resolve a symbolic RCC register name to its byte offset within THIS chip
    /// family's RCC register map. Returns `None` for a name the active family
    /// does not have. The offsets deliberately differ between families
    /// (F1 apb1enr@0x1C vs L4 apb1enr@0x58; L0 crrcr@0x08 vs WB crrcr@0x98),
    /// which is exactly why this lives on the family-aware model rather than in
    /// the bus.
    ///
    /// Used by the bus to map a peripheral's `clock:` declaration onto the real
    /// RCC registers it must read for the gate check
    /// ([`crate::bus::SystemBus::is_peripheral_clocked`]).
    ///
    /// Two kinds of name resolve here, because silicon withholds a peripheral's
    /// clock for two kinds of reason:
    ///
    /// * **peripheral-enable registers** — "ahbenr", "apb1enr", "apb2enr", … —
    ///   the bus clock gate;
    /// * **clock-source registers** — "cr", "crrcr", … — for a peripheral fed by
    ///   its own kernel clock, whose *ready* bit says the source is actually
    ///   running (the STM32L0 RNG is dead without HSI48 regardless of
    ///   AHBENR.RNGEN).
    ///
    /// Source-register names are added per family only where the offset is
    /// sourced from the vendor SVD vendored in `tests/fixtures/real_world/`; an
    /// unsourced family returns `None`, which the bus turns into a loud config
    /// error rather than a gate that silently never fires.
    pub fn rcc_reg_offset(&self, reg: &str) -> Option<u64> {
        let r = reg.trim().to_ascii_lowercase();
        // Clock-SOURCE registers, per family. Enable registers follow below.
        match (self, r.as_str()) {
            // L0: CR@0x00, CRRCR@0x08 (HSI48ON bit0 → HSI48RDY bit1).
            // tests/fixtures/real_world/stm32l073.svd.
            (Self::Stm32L0(_), "cr") => return Some(0x00),
            (Self::Stm32L0(_), "crrcr") => return Some(0x08),
            // V2 clock sources: CR@0x00 everywhere; CRRCR@0x98 on WB/WBA
            // (RM0434/RM0493) but 0x14 on U5 (RM0456 §11.8.6).
            (Self::Stm32V2(_), "cr") => return Some(0x00),
            (Self::Stm32V2(v2), "crrcr") => return Some(if v2.u5_cr_ready { 0x14 } else { 0x98 }),
            _ => {}
        }
        match self {
            // F1: AHBENR@0x14, APB2ENR@0x18, APB1ENR@0x1C (RM0008 §7.3).
            Self::Stm32F1(_) => match r.as_str() {
                "ahbenr" | "ahb1enr" => Some(0x14),
                "apb2enr" => Some(0x18),
                "apb1enr" | "apb1enr1" => Some(0x1C),
                _ => None,
            },
            // F0: AHBENR@0x14, APB2ENR@0x18, APB1ENR@0x1C (RM0360 §6.4.6-8);
            // `cr2` resolves the HSI14 ADC clock source (HSI14RDY bit 1).
            Self::Stm32F0(_) => match r.as_str() {
                "cr" => Some(0x00),
                "cr2" => Some(0x34),
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
            // G4: AHB1ENR@0x48, AHB2ENR@0x4C, APB1ENR1@0x58, APB1ENR2@0x5C,
            // APB2ENR@0x60 (RM0440 §7.4 / CMSIS stm32g474xx.h).
            Self::Stm32G4(_) => match r.as_str() {
                "ahbenr" | "ahb1enr" => Some(0x48),
                "ahb2enr" => Some(0x4C),
                "apb1enr" | "apb1enr1" => Some(0x58),
                "apb1enr2" => Some(0x5C),
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
            // U5 (`stm32v2`, RM0456): the full `V2U5EnrMap` block plus the
            // clock-source names the U5 chip yaml gates use (bdcr for the RTC
            // kernel clock, csr if a gate ever needs the reset-flag register).
            // Read straight off the map the register decode uses, so a gate can
            // never resolve to an offset the decode does not honour.
            Self::Stm32V2(v2) if v2.u5_cr_ready => {
                let m = V2U5EnrMap::U5;
                match r.as_str() {
                    "cfgr1" => Some(0x1C),
                    "ahb1enr" => Some(m.ahb1enr),
                    "ahbenr" | "ahb2enr" | "ahb2enr1" => Some(m.ahb2enr1),
                    "ahb2enr2" => Some(m.ahb2enr2),
                    "ahb3enr" => Some(m.ahb3enr),
                    "apb1enr" | "apb1enr1" => Some(m.apb1enr1),
                    "apb1enr2" => Some(m.apb1enr2),
                    "apb2enr" => Some(m.apb2enr),
                    "apb3enr" => Some(m.apb3enr),
                    "ahb1rstr" => Some(m.ahb1rstr),
                    "ahb2rstr" | "ahb2rstr1" => Some(m.ahb2rstr1),
                    "ahb2rstr2" => Some(m.ahb2rstr2),
                    "ahb3rstr" => Some(m.ahb3rstr),
                    "apb1rstr" | "apb1rstr1" => Some(m.apb1rstr1),
                    "apb1rstr2" => Some(m.apb1rstr2),
                    "apb2rstr" => Some(m.apb2rstr),
                    "apb3rstr" => Some(m.apb3rstr),
                    "bdcr" => Some(0xF0),
                    "csr" => Some(0xF4),
                    _ => None,
                }
            }
            // G0: IOPENR@0x34, AHBENR@0x38, APBENR1@0x3C, APBENR2@0x40
            // (RM0444 §5.4 / CMSIS stm32g071xx.h RCC_TypeDef). `cr` resolves
            // the HSI16 kernel-clock source (HSIRDY bit 10) for peripherals
            // gated on a running source rather than only a bus enable.
            Self::Stm32G0(_) => match r.as_str() {
                "cr" => Some(0x00),
                "iopenr" => Some(0x34),
                "ahbenr" => Some(0x38),
                "apbenr1" | "apb1enr" | "apb1enr1" => Some(0x3C),
                "apbenr2" | "apb2enr" => Some(0x40),
                "bdcr" => Some(0x5C),
                "csr" => Some(0x60),
                _ => None,
            },
            // WB (RM0434, 0x4C/0x58/0x60) and WBA (RM0493, H5-style
            // 0x8C/0x9C/0xA4): the six shared slots, read off the instance's
            // own map. `cfgr1` is WBA RCC_CFGR1@0x1C (WB has RCC_CIFR there
            // and nothing declares it).
            Self::Stm32V2(v2) => match r.as_str() {
                "cfgr1" if v2.synthesize_wba_rclk_pre_rdy => Some(0x1C),
                "ahbenr" | "ahb2enr" => Some(v2.map.ahbenr),
                "apb1enr" | "apb1enr1" | "apb1lenr" => Some(v2.map.apb1enr),
                "apb2enr" => Some(v2.map.apb2enr),
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
            Self::Stm32G4(r) => r,
            Self::Stm32G0(r) => r,
            Self::Stm32F0(r) => r,
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
            Self::Stm32G4(r) => r,
            Self::Stm32G0(r) => r,
            Self::Stm32F0(r) => r,
        }
    }
}

impl crate::Peripheral for Rcc {
    /// Record a supply reset in RCC_CSR. Modelled on the F4 layout; the other
    /// families' CSR reset flags stay as they were (plain storage).
    fn on_supply_reset(&mut self, cause: crate::power::SupplyResetCause) {
        if let Self::Stm32F4(rcc) = self {
            rcc.on_supply_reset(cause);
        }
    }

    fn stm32_uart_pad_af_override(&self) -> Option<u8> {
        matches!(self, Self::Stm32G0(_)).then_some(1)
    }

    /// The RCC is this chip's clock controller: resolve `clock:` register names
    /// through the family map that already exists for them.
    fn clock_gate_reg_offset(&self, name: &str) -> Option<u64> {
        self.rcc_reg_offset(name)
    }

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

    /// RCC registers are 32-bit, and a word store must reach the model as ONE
    /// write. The trait default splits a store into four byte
    /// read-modify-writes, which un-does write-1-to-clear / self-clearing
    /// bits: firmware's `RCC->CSR |= RCC_CSR_RMVF` compiles to a store whose
    /// RMVF byte (2) lands *before* the top byte carrying the reset flags (3),
    /// so the flags RMVF just cleared would be re-latched. Aligned stores go
    /// straight to `write_reg`; an unaligned store (never issued to Device
    /// memory, but kept correct) falls back to the per-byte path.
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset % 4 == 0 {
            self.model_mut().write_reg(offset, value);
            return Ok(());
        }
        for (i, byte) in value.to_le_bytes().iter().enumerate() {
            self.write(offset + i as u64, *byte)?;
        }
        Ok(())
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
    // name to a concrete offset via the family-aware `rcc_reg_offset`.
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

    /// Every family Zephyr drives auto-acks an LSI enable→ready pair, at the
    /// per-family offset. Zephyr's RTC/LSI clock init spins on the ready bit.
    ///
    /// The classic V2 (WB/WBA, RM0434/RM0493) LSI pair is CSR@0x94 bit0→bit1.
    /// U5 (RM0456) moves LSI into RCC_BDCR@0xF0 at bits 26/27 (HAL
    /// `RCC_BDCR_LSION`/`RCC_BDCR_LSIRDY`); its CSR@0xF4 is only reset flags
    /// and MSI ranges. WBA's BDCR1@0xF0 triple is exercised in
    /// `v2_wb_wba_backup_and_switch_gates`.
    #[test]
    fn lsi_ready_auto_acks_per_family() {
        // (layout, reg, on_bit, rdy_bit)
        for (layout, csr, on, rdy) in [
            (RccRegisterLayout::Stm32F1, 0x24u64, 0u32, 1u32),
            (RccRegisterLayout::Stm32F4, 0x74, 0, 1),
            (RccRegisterLayout::Stm32L0, 0x50, 0, 1),
            (RccRegisterLayout::Stm32L4, 0x94, 0, 1),
            (RccRegisterLayout::Stm32Wb, 0x94, 0, 1),
            (RccRegisterLayout::Stm32V2, 0xF0, 26, 27),
        ] {
            let mut rcc = Rcc::new_with_layout(layout);
            rcc.write_u32(csr, 1 << on).unwrap(); // LSI ON
            assert_eq!(
                rcc.read_u32(csr).unwrap() & ((1 << on) | (1 << rdy)),
                (1 << on) | (1 << rdy),
                "{:?} LSI@{:#x} must set RDY bit {}",
                layout,
                csr,
                rdy
            );
        }
    }

    /// L4 gates the kernel clock on HSI16RDY, and every V2 flavour (classic
    /// WB/WBA and the U5 remap) reports it at CR bit 10 (HSION at bit 8) —
    /// unlike the MSI bit-1 slot of the older families.
    #[test]
    fn hsi16_ready_at_bit10() {
        for layout in [
            RccRegisterLayout::Stm32L4,
            RccRegisterLayout::Stm32Wb,
            RccRegisterLayout::Stm32Wba,
            RccRegisterLayout::Stm32V2,
        ] {
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

    /// The classic V2 layouts (G4/WB/WBA, RM0440/RM0434/RM0493) expose HSI48
    /// via CRRCR@0x98: HSI48ON→HSI48RDY. U5 (RM0456) moves CRRCR to 0x14 —
    /// 0x98 is not a register there, and writing it must not latch HSI48RDY.
    #[test]
    fn v2_hsi48_ready() {
        let mut classic = Rcc::new_with_layout(RccRegisterLayout::Stm32Wb);
        classic.write_u32(0x98, 1).unwrap();
        assert_eq!(
            classic.read_u32(0x98).unwrap() & 0x3,
            0x3,
            "WB CRRCR@0x98 must set HSI48RDY"
        );

        let mut u5 = Rcc::new_with_layout(RccRegisterLayout::Stm32V2);
        u5.write_u32(0x14, 1).unwrap();
        assert_eq!(
            u5.read_u32(0x14).unwrap() & 0x3,
            0x3,
            "U5 CRRCR@0x14 must set HSI48RDY"
        );
        u5.write_u32(0x98, 1).unwrap();
        assert_ne!(
            u5.read_u32(0x98).unwrap() & 0x3,
            0x3,
            "0x98 is not CRRCR on U5 (RM0456 CRRCR@0x14)"
        );
    }

    /// U5 (RM0456/RM0482) packs the ready flags at H5-style CR positions —
    /// MSISRDY bit2, MSIKRDY bit5, HSI48RDY bit13 — not WB/WBA's classic
    /// MSIRDY bit1. The NUCLEO-U575ZI-Q variant's `SystemClock_Config` enables
    /// HSI48 (`RCC_OscInitStruct.HSI48State = RCC_HSI48_ON`) and the Cube HAL
    /// polls `RCC->CR.HSI48RDY` with a timeout; when the model never latches
    /// bit13, `HAL_RCC_OscConfig` returns HAL_TIMEOUT and the core's
    /// `Error_Handler()` spins forever. This was the first of the two
    /// sequential Arduino-matrix L0 boot blockers; once fixed, boot advanced
    /// to the undeclared CRS window (see configs/chips/stm32u575.yaml).
    #[test]
    fn v2_u5_cr_ready_flags() {
        let mut rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32V2);

        // SVD reset 0x35: MSISON|MSISRDY|MSIKON|MSIKRDY.
        assert_eq!(rcc.read_u32(0x00).unwrap(), 0x35, "U5 CR reset");

        // HSI48ON (bit12) → HSI48RDY (bit13); bit1 is MSIKERON, not a RDY.
        rcc.write_u32(0x00, 0x35 | (1 << 12)).unwrap();
        let cr = rcc.read_u32(0x00).unwrap();
        assert_ne!(cr & (1 << 13), 0, "HSI48RDY follows HSI48ON");
        assert_eq!(cr & (1 << 1), 0, "U5 bit1 is MSIKERON, not MSIRDY");

        // Clearing HSI48ON drops HSI48RDY.
        rcc.write_u32(0x00, cr & !(1 << 12)).unwrap();
        assert_eq!(rcc.read_u32(0x00).unwrap() & (1 << 13), 0);

        // MSISRDY bit2 follows MSISON bit0; MSIKRDY bit5 follows MSIKON bit4.
        rcc.write_u32(0x00, 1 << 0).unwrap();
        let cr = rcc.read_u32(0x00).unwrap();
        assert_ne!(cr & (1 << 2), 0, "MSISRDY follows MSISON");
        assert_eq!(cr & (1 << 1), 0, "U5 bit1 is MSIKERON");
        rcc.write_u32(0x00, 1 << 4).unwrap();
        let cr = rcc.read_u32(0x00).unwrap();
        assert_ne!(cr & (1 << 5), 0, "MSIKRDY follows MSIKON");
        assert_eq!(cr & (1 << 2), 0, "MSISRDY drops when MSISON is off");
    }

    /// WB (RM0434 §6.4) keeps the classic backup domain: RCC_BDCR@0x90 acks
    /// LSEON→LSERDY, and RCC_CFGR@0x08 follows SW→SWS only once the requested
    /// source is ready in CR. WBA (RM0493) has its backup enables in
    /// RCC_BDCR1@0xF0 — LSION(0)→LSIRDY(1), LSESYSEN(7)→LSESYSRDY(11) and the
    /// bit26→bit27 pair — and switches SYSCLK through RCC_CFGR1@0x1C.
    ///
    /// RCC_PLL1CFGR @ 0x28 differs by family. RM0456 (U5) defines no field at
    /// bits 19-31, so it is plain storage. RM0493 §7.7.13 (WBA) defines bit20
    /// `PLL1RCLKPRE` with read-only status bit22 `PLL1RCLKPRERDY`, which the
    /// Cube HAL and Zephyr's `clock_stm32_ll_wba.c` poll after clearing bit20:
    /// the `stm32wba` layout synthesizes `RDY = !PRE`, `stm32v2` does not.
    #[test]
    fn v2_wb_wba_backup_and_switch_gates() {
        // WB: classic BDCR@0x90 + CFGR@0x08 source gating.
        let mut wb = Rcc::new_with_layout(RccRegisterLayout::Stm32Wb);
        wb.write_u32(0x90, 1).unwrap(); // BDCR LSEON
        assert_eq!(wb.read_u32(0x90).unwrap() & 0x3, 0x3, "WB LSERDY");
        wb.write_u32(0x08, 0x3).unwrap(); // SW=PLL, PLL off
        assert_eq!(
            (wb.read_u32(0x08).unwrap() >> 2) & 0x3,
            0x0,
            "WB SWS holds while PLL not ready"
        );
        wb.write_u32(0x00, wb.read_u32(0x00).unwrap() | (1 << 24))
            .unwrap(); // PLLON → PLLRDY
        wb.write_u32(0x08, 0x3).unwrap();
        assert_eq!(
            (wb.read_u32(0x08).unwrap() >> 2) & 0x3,
            0x3,
            "WB SWS follows SW once PLL ready"
        );

        // WBA: BDCR1@0xF0 backup pairs + CFGR1@0x1C source gating.
        let mut wba = Rcc::new_with_layout(RccRegisterLayout::Stm32Wba);
        wba.write_u32(0xF0, (1 << 0) | (1 << 7) | (1 << 26))
            .unwrap();
        let bdcr1 = wba.read_u32(0xF0).unwrap();
        for rdy in [1u32, 11, 27] {
            assert_ne!(bdcr1 & (1 << rdy), 0, "WBA BDCR1 rdy bit {rdy}");
        }
        // CFGR1 SW=PLL1R (0b11) is gated on PLLRDY: with the PLL off the
        // switch holds; enabling PLL1 (CR bit24 → PLLRDY bit25) lets it
        // complete.
        wba.write_u32(0x1C, 0x3).unwrap();
        assert_eq!(
            (wba.read_u32(0x1C).unwrap() >> 2) & 0x3,
            0x0,
            "WBA SWS holds while PLL not ready"
        );
        wba.write_u32(0x00, 1 << 24).unwrap(); // PLL1ON → PLL1RDY
        wba.write_u32(0x1C, 0x3).unwrap();
        assert_eq!(
            (wba.read_u32(0x1C).unwrap() >> 2) & 0x3,
            0x3,
            "WBA SWS follows SW once PLL ready"
        );

        // U5 (`stm32v2`): plain storage. This value has bit20 clear, which the
        // always-on synthetic ack would have corrupted with a forced bit22.
        let mut u5 = Rcc::new_with_layout(RccRegisterLayout::Stm32V2);
        u5.write_u32(0x28, 0x0004_1401).unwrap();
        assert_eq!(
            u5.read_u32(0x28).unwrap(),
            0x0004_1401,
            "stm32v2 PLL1CFGR is storage (RM0456: bits 19-31 reserved)"
        );

        // WBA (`stm32wba`): `PLL1RCLKPRERDY` (bit22, read-only) reads 1 once
        // `PLL1RCLKPRE` (bit20) is clear and 0 while it is set; the stored
        // bits survive both reads (RM0493 §7.7.13).
        let mut wba = Rcc::new_with_layout(RccRegisterLayout::Stm32Wba);
        wba.write_u32(0x28, 0x0004_1400).unwrap(); // PRE=0, M/REN as stored
        assert_eq!(
            wba.read_u32(0x28).unwrap(),
            0x0004_1400 | (1 << 22),
            "stm32wba PRERDY reads 1 once PRE is cleared"
        );
        wba.write_u32(0x28, 0x0010_1400).unwrap(); // PRE=1
        assert_eq!(
            wba.read_u32(0x28).unwrap(),
            0x0010_1400,
            "stm32wba PRERDY reads 0 while PRE is set"
        );
    }

    /// U5 (RM0456) remaps the backup/oscillator half of the V2 clock tree:
    /// ICSCR1/2/3 at 0x08/0x0C/0x10 (`icscr` re-used for ICSCR1), CRRCR at
    /// 0x14, AHB2ENR2 at 0x90, AHB3ENR at 0x94, BDCR at 0xF0 and CSR at 0xF4.
    /// The classic V2 offsets must NOT answer for these registers: 0x94 is
    /// AHB3ENR (not CSR), 0x98 is reserved (not CRRCR) and 0x90 is AHB2ENR2
    /// (not BDCR).
    #[test]
    fn v2_u5_backup_and_enable_storage() {
        // ICSCR1 is seeded to the vendored-SVD reset 0x4400_0000 (RM0456);
        // ICSCR2/ICSCR3 to their yaml resets.
        let mut rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32V2);
        assert_eq!(rcc.read_u32(0x08).unwrap(), 0x4400_0000, "ICSCR1 reset");
        assert_eq!(rcc.read_u32(0x0C).unwrap(), 0x0008_4210, "ICSCR2 reset");
        assert_eq!(rcc.read_u32(0x10).unwrap(), 0x0010_0000, "ICSCR3 reset");
        for (off, val) in [
            (0x08u64, 0x1234_5678u32),
            (0x0C, 0x0000_001F),
            (0x10, 0x001F_0000),
            (0x90, 0x0000_0004),
            (0x94, 0x0000_0008),
        ] {
            rcc.write_u32(off, val).unwrap();
            assert_eq!(rcc.read_u32(off).unwrap(), val, "U5 storage @ {off:#x}");
        }

        // BDCR@0xF0 carries all three U5 backup handshakes (RM0456 §11.8.40
        // + CubeU5 `stm32u5xx_hal_rcc.c`): LSEON bit0 → LSERDY bit1,
        // LSESYSEN bit7 → LSESYSRDY bit11 (both polled by HAL_RCC_OscConfig)
        // and LSION bit26 → LSIRDY bit27 (U5 keeps LSI in BDCR, not CSR).
        rcc.write_u32(0xF0, (1 << 0) | (1 << 7) | (1 << 26))
            .unwrap();
        let bdcr = rcc.read_u32(0xF0).unwrap();
        for rdy in [1u32, 11, 27] {
            assert_ne!(bdcr & (1 << rdy), 0, "U5 BDCR rdy bit {rdy}");
        }
        rcc.write_u32(0xF0, 0).unwrap();
        assert_eq!(
            rcc.read_u32(0xF0).unwrap() & ((1 << 1) | (1 << 11) | (1 << 27)),
            0,
            "U5 BDCR ready bits drop when the enables clear"
        );

        // CSR@0xF4 is storage on U5 (reset flags + MSIS/MSIK ranges); it has
        // no LSION/LSIRDY fields, so writing bit0 must not latch an LSI ack.
        rcc.write_u32(0xF4, 0x0C00_0000).unwrap();
        assert_eq!(rcc.read_u32(0xF4).unwrap(), 0x0C00_0000, "U5 CSR storage");
        rcc.write_u32(0xF4, 0x1).unwrap();
        assert_eq!(rcc.read_u32(0xF4).unwrap(), 0x1, "U5 CSR bit0 is not LSION");

        // 0x94 carries AHB3ENR storage, so its bit1 must not become LSIRDY.
        rcc.write_u32(0x94, 0).unwrap();
        assert_eq!(rcc.read_u32(0x94).unwrap(), 0, "0x94 is not CSR on U5");
    }

    /// U5 (RM0456 §11.8) RCC_CFGR1@0x1C: SW 00 MSIS, 01 HSI16, 10 HSE,
    /// 11 PLL1, gated on MSISRDY(CR bit2), HSIRDY(10), HSERDY(17),
    /// PLL1RDY(25). The reset CR (0x35) already has MSIS ready.
    #[test]
    fn v2_u5_cfgr1_switches_when_source_ready() {
        let mut rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32V2);

        // PLL1 off: SW=11 holds SWS at 00 (MSIS).
        rcc.write_u32(0x1C, 0x3).unwrap();
        assert_eq!(
            (rcc.read_u32(0x1C).unwrap() >> 2) & 0x3,
            0x0,
            "SWS holds while PLL1 not ready"
        );
        rcc.write_u32(0x00, rcc.read_u32(0x00).unwrap() | (1 << 24))
            .unwrap();
        rcc.write_u32(0x1C, 0x3).unwrap();
        assert_eq!(
            (rcc.read_u32(0x1C).unwrap() >> 2) & 0x3,
            0x3,
            "SWS=PLL1 once PLL1RDY"
        );

        // HSI16 off: SW=01 holds SWS at 11; HSION latches HSIRDY bit10.
        rcc.write_u32(0x1C, 0x1).unwrap();
        assert_eq!((rcc.read_u32(0x1C).unwrap() >> 2) & 0x3, 0x3);
        rcc.write_u32(0x00, rcc.read_u32(0x00).unwrap() | (1 << 8))
            .unwrap();
        rcc.write_u32(0x1C, 0x1).unwrap();
        assert_eq!(
            (rcc.read_u32(0x1C).unwrap() >> 2) & 0x3,
            0x1,
            "SWS=HSI16 once HSIRDY"
        );

        // Back to MSIS: with MSISON clear, MSISRDY (bit2) is clear, so SWS
        // holds at 01. Setting MSISON lets SWS=00 complete.
        let cr = rcc.read_u32(0x00).unwrap();
        rcc.write_u32(0x00, cr & !(1 << 0)).unwrap(); // MSISON=0
        rcc.write_u32(0x1C, 0x0).unwrap();
        assert_eq!(
            (rcc.read_u32(0x1C).unwrap() >> 2) & 0x3,
            0x1,
            "SWS holds while MSISRDY clear (bit2, not classic bit1)"
        );
        rcc.write_u32(0x00, rcc.read_u32(0x00).unwrap() | (1 << 0))
            .unwrap();
        rcc.write_u32(0x1C, 0x0).unwrap();
        assert_eq!(
            (rcc.read_u32(0x1C).unwrap() >> 2) & 0x3,
            0x0,
            "SWS=MSIS once MSISRDY"
        );
    }

    /// U5's 0x08 is ICSCR1 storage — the classic G4/WB CFGR arm (SWS gating
    /// and the forced bits16-18 applied flags) must not run on it.
    #[test]
    fn v2_u5_0x08_is_icscr1_not_cfgr() {
        let mut rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32V2);
        rcc.write_u32(0x08, 0x0001_0003).unwrap();
        assert_eq!(
            rcc.read_u32(0x08).unwrap(),
            0x0001_0003,
            "ICSCR1 must round-trip untouched (no bits16-18 injection, no SWS)"
        );
    }

    /// STM32U5 (RM0456) puts PLL1CFGR at 0x28, PLL1DIVR at 0x34 and PLL1FRACR
    /// at 0x38 — plain read/write storage, matching the vendored
    /// `configs/peripherals/stm32u575/rcc.yaml` (decimal offsets 40/52/56). The
    /// old synthetic bit20↔bit22 request/ack read arm at 0x28 returned a value
    /// HAL never wrote, so any PLL1CFGR read-back (or DIVR/FRACR read) came
    /// back corrupt.
    #[test]
    fn v2_u5_pll1_registers_round_trip_and_ready() {
        let mut rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32V2);

        // Reset state first: PLL1DIVR is seeded to the vendored-SVD reset
        // 0x01010280 (N=0x80, P=1, Q=1, R=2); PLL1CFGR/PLL1FRACR reset to 0.
        assert_eq!(rcc.read_u32(0x28).unwrap(), 0, "PLL1CFGR reset");
        assert_eq!(rcc.read_u32(0x34).unwrap(), 0x0101_0280, "PLL1DIVR reset");
        assert_eq!(rcc.read_u32(0x38).unwrap(), 0, "PLL1FRACR reset");

        // PLL1CFGR: source MSIS, M=1, REN|PEN, RGE range 0.
        let pll1cfgr: u32 = 0x0000_1401;
        rcc.write_u32(0x28, pll1cfgr).unwrap();
        assert_eq!(
            rcc.read_u32(0x28).unwrap(),
            pll1cfgr,
            "PLL1CFGR must read back"
        );

        // Distinct non-reset values, so a hardwired reset default or a missing
        // decode arm cannot pass by coincidence.
        rcc.write_u32(0x34, 0x0303_0509).unwrap();
        assert_eq!(rcc.read_u32(0x34).unwrap(), 0x0303_0509, "PLL1DIVR");
        rcc.write_u32(0x38, 0x0000_8000).unwrap();
        assert_eq!(rcc.read_u32(0x38).unwrap(), 0x0000_8000, "PLL1FRACR");

        // CR.PLL1ON (bit 24) must gate CR.PLL1RDY (bit 25) like the classic path.
        rcc.write_u32(0x00, 1 << 24).unwrap();
        assert_ne!(
            rcc.read_u32(0x00).unwrap() & (1 << 25),
            0,
            "PLL1RDY follows PLL1ON"
        );
        rcc.write_u32(0x00, 0).unwrap();
        assert_eq!(
            rcc.read_u32(0x00).unwrap() & (1 << 25),
            0,
            "PLL1RDY follows PLL1ON off"
        );
    }

    /// U5 (RM0456 §11.8.12-19) full enable/reset block. Every offset and reset
    /// below is read off the vendored `configs/peripherals/stm32u575/rcc.yaml`
    /// (generated from the SVD): AHB1RSTR@0x60, AHB2RSTR1@0x64,
    /// AHB2RSTR2@0x68, AHB3RSTR@0x6C, APB1RSTR1@0x74, APB1RSTR2@0x78,
    /// APB2RSTR@0x7C, APB3RSTR@0x80, AHB1ENR@0x88, AHB2ENR1@0x8C,
    /// AHB2ENR2@0x90, AHB3ENR@0x94, APB1ENR1@0x9C, APB1ENR2@0xA0,
    /// APB2ENR@0xA4, APB3ENR@0xA8. The reset block is the part the old model
    /// aliased onto the H5/WBA slots (0x6C/0x7C/0x84); 0x84 is reserved here.
    #[test]
    fn v2_u5_enr_block_reads_writes() {
        let mut rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32V2);

        // SVD reset values: AHB1ENR 0xD020_0100 (FLASHEN|GPU2DEN|BKPSRAMEN|
        // DCACHE1EN|SRAM1EN), AHB2ENR1 0xC000_0000 (SRAM2/3EN), AHB2ENR2 and
        // AHB3ENR 0x8000_0000 (SRAM5/SRAM4EN); every other ENR/RSTR is 0.
        assert_eq!(rcc.read_u32(0x88).unwrap(), 0xD020_0100, "AHB1ENR reset");
        assert_eq!(rcc.read_u32(0x8C).unwrap(), 0xC000_0000, "AHB2ENR1 reset");
        assert_eq!(rcc.read_u32(0x90).unwrap(), 0x8000_0000, "AHB2ENR2 reset");
        assert_eq!(rcc.read_u32(0x94).unwrap(), 0x8000_0000, "AHB3ENR reset");
        for off in [
            0x60u64, 0x64, 0x68, 0x6C, 0x74, 0x78, 0x7C, 0x80, 0x9C, 0xA0, 0xA4, 0xA8,
        ] {
            assert_eq!(rcc.read_u32(off).unwrap(), 0, "reset @ {off:#x}");
        }

        // A distinct implemented bit per register: a mis-decoded arm that
        // routes to another register cannot pass the read-back.
        for (off, val) in [
            (0x60u64, 0x0000_0001u32), // AHB1RSTR.GPDMA1RST
            (0x64, 0x0000_0400),       // AHB2RSTR1.ADC12RST
            (0x68, 0x0000_0010),       // AHB2RSTR2.OCTOSPI1RST
            (0x6C, 0x0000_0020),       // AHB3RSTR.ADC4RST
            (0x74, 0x0000_0001),       // APB1RSTR1.TIM2RST
            (0x78, 0x0000_0002),       // APB1RSTR2.I2C4RST
            (0x7C, 0x0000_0800),       // APB2RSTR.TIM1RST
            (0x80, 0x0000_0002),       // APB3RSTR.SYSCFGRST
            (0x88, 0x0000_0001),       // AHB1ENR.GPDMA1EN
            (0x8C, 0x0000_0001),       // AHB2ENR1.GPIOAEN
            (0x90, 0x0000_0001),       // AHB2ENR2.FSMCEN
            (0x94, 0x0000_0004),       // AHB3ENR.PWREN
            (0x9C, 0x0000_0020),       // APB1ENR1.TIM6EN
            (0xA0, 0x0000_0002),       // APB1ENR2.I2C4EN
            (0xA4, 0x0000_4000),       // APB2ENR.USART1EN
            (0xA8, 0x0000_0040),       // APB3ENR.LPUART1EN
        ] {
            rcc.write_u32(off, val).unwrap();
            assert_eq!(rcc.read_u32(off).unwrap(), val, "U5 ENR/RSTR @ {off:#x}");
        }

        // 0x84 is reserved on U5 (APB3RSTR@0x80, AHB1ENR@0x88) — it must not
        // answer as the H5/WBA-style APB2RSTR slot.
        rcc.write_u32(0x84, 0xFFFF_FFFF).unwrap();
        assert_eq!(rcc.read_u32(0x84).unwrap(), 0, "0x84 is reserved on U5");
    }

    /// U5 PLL2/PLL3 (RM0456 §11.8.8-11): PLL2CFGR@0x2C, PLL3CFGR@0x30,
    /// PLL2DIVR@0x3C, PLL2FRACR@0x40, PLL3DIVR@0x44, PLL3FRACR@0x48 — plain
    /// storage with the SVD DIVR reset 0x0101_0280 — plus the CR ready pairs
    /// PLL2ON(26)→PLL2RDY(27) and PLL3ON(28)→PLL3RDY(29).
    #[test]
    fn v2_u5_pll23_round_trip_and_ready() {
        let mut rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32V2);

        assert_eq!(rcc.read_u32(0x2C).unwrap(), 0, "PLL2CFGR reset");
        assert_eq!(rcc.read_u32(0x30).unwrap(), 0, "PLL3CFGR reset");
        assert_eq!(rcc.read_u32(0x3C).unwrap(), 0x0101_0280, "PLL2DIVR reset");
        assert_eq!(rcc.read_u32(0x44).unwrap(), 0x0101_0280, "PLL3DIVR reset");
        assert_eq!(rcc.read_u32(0x40).unwrap(), 0, "PLL2FRACR reset");
        assert_eq!(rcc.read_u32(0x48).unwrap(), 0, "PLL3FRACR reset");

        for (off, val) in [
            (0x2Cu64, 0x0000_1403u32), // PLL2SRC/MSIS, M=1, REN|QEN|PEN
            (0x30, 0x0000_1405),
            (0x3C, 0x0303_0509),
            (0x40, 0x0000_8000),
            (0x44, 0x0404_060A),
            (0x48, 0x0000_4000),
        ] {
            rcc.write_u32(off, val).unwrap();
            assert_eq!(rcc.read_u32(off).unwrap(), val, "U5 PLL2/3 @ {off:#x}");
        }
        assert_eq!(
            rcc.read_u32(0x28).unwrap(),
            0,
            "PLL1CFGR must not alias PLL2/3"
        );

        rcc.write_u32(0x00, 1 << 26).unwrap();
        assert_ne!(rcc.read_u32(0x00).unwrap() & (1 << 27), 0, "PLL2RDY");
        rcc.write_u32(0x00, 1 << 28).unwrap();
        assert_ne!(rcc.read_u32(0x00).unwrap() & (1 << 29), 0, "PLL3RDY");
        rcc.write_u32(0x00, 0).unwrap();
        assert_eq!(
            rcc.read_u32(0x00).unwrap() & ((1 << 27) | (1 << 29)),
            0,
            "PLL2/3 RDY drop when ON clears"
        );
    }

    /// U5 CFGR2@0x20 (reset 0x6000 = DPRE ÷16) and CFGR3@0x24 storage, plus
    /// the clock-interrupt block (RM0456 §11.8.20-22): CIER@0x50 enable
    /// storage, CIFR@0x54 flag storage, CICR@0x58 write-1-to-clear against
    /// CIFR and read as 0 (write-only).
    #[test]
    fn v2_u5_cfgr23_and_ci() {
        let mut rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32V2);

        assert_eq!(rcc.read_u32(0x20).unwrap(), 0x0000_6000, "CFGR2 reset");
        assert_eq!(rcc.read_u32(0x24).unwrap(), 0, "CFGR3 reset");
        rcc.write_u32(0x20, 0x000F_7700).unwrap();
        assert_eq!(rcc.read_u32(0x20).unwrap(), 0x000F_7700, "CFGR2 round-trip");
        rcc.write_u32(0x24, 0x0003_0070).unwrap();
        assert_eq!(rcc.read_u32(0x24).unwrap(), 0x0003_0070, "CFGR3 round-trip");

        rcc.write_u32(0x50, 0x0000_1DFF).unwrap();
        assert_eq!(rcc.read_u32(0x50).unwrap(), 0x0000_1DFF, "CIER round-trip");
        rcc.write_u32(0x54, 0x0000_1D01).unwrap();
        assert_eq!(rcc.read_u32(0x54).unwrap(), 0x0000_1D01, "CIFR round-trip");

        rcc.write_u32(0x58, 0x0000_0001).unwrap();
        assert_eq!(
            rcc.read_u32(0x54).unwrap(),
            0x0000_1D00,
            "CICR clears only the bits written as 1"
        );
        assert_eq!(rcc.read_u32(0x58).unwrap(), 0, "CICR reads 0");
        rcc.write_u32(0x58, 0xFFFF_FFFF).unwrap();
        assert_eq!(rcc.read_u32(0x54).unwrap(), 0, "CICR all clears CIFR");
    }

    /// U5 RCC_CSR@0xF4 (RM0456 §11.8.41): reset 0x0C00_4400 (MSIKSRANGE=4,
    /// MSISSRANGE=4, PINRSTF|BORRSTF). RMVF (bit 23) is write-1-to-clear for
    /// the reset flags (bits 25-31) and self-clearing; the MSI range fields
    /// stay ordinary storage.
    #[test]
    fn csr_rmvf_write_one_clears() {
        let mut rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32V2);
        assert_eq!(rcc.read_u32(0xF4).unwrap(), 0x0C00_4400, "U5 CSR reset");

        // A write without RMVF leaves the flags alone; RMVF reads back 0.
        rcc.write_u32(0xF4, 0x0C00_5500).unwrap();
        assert_eq!(rcc.read_u32(0xF4).unwrap(), 0x0C00_5500, "CSR storage");

        // RMVF clears OBLRSTF..LPWRRSTF, keeps the range fields.
        rcc.write_u32(0xF4, 0x0C00_5500 | (1 << 23)).unwrap();
        assert_eq!(
            rcc.read_u32(0xF4).unwrap(),
            0x0000_5500,
            "RMVF must clear the reset flags and read back 0"
        );

        rcc.write_u32(0xF4, 0x0C00_4400).unwrap();
        assert_eq!(rcc.read_u32(0xF4).unwrap(), 0x0C00_4400);
    }

    /// The WBA ready-status switch is fixed configuration, not simulated state:
    /// it must stay out of the snapshot like the adjacent `map` field, while
    /// the PLL1/ICSCR/enable storage registers are state and must appear.
    #[test]
    fn v2_rcc_snapshot_skips_ready_status_config_keeps_pll1_state() {
        let mut rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32V2);
        rcc.write_u32(0x28, 0x0004_1400).unwrap();
        rcc.write_u32(0x34, 0x0303_0509).unwrap();
        rcc.write_u32(0x38, 0x0000_8000).unwrap();
        rcc.write_u32(0x0C, 0x0000_001F).unwrap();
        rcc.write_u32(0x10, 0x001F_0000).unwrap();
        rcc.write_u32(0x90, 0x0000_0004).unwrap();
        rcc.write_u32(0x94, 0x0000_0008).unwrap();

        let snap = rcc.snapshot();
        assert!(
            snap.get("synthesize_wba_rclk_pre_rdy").is_none(),
            "fixed config flag must not be serialized: {snap}"
        );
        assert!(
            snap.get("u5_cr_ready").is_none(),
            "U5 CR ready-layout flag must not be serialized: {snap}"
        );
        assert_eq!(snap["pll1cfgr"], 0x0004_1400u32);
        assert_eq!(snap["pll1divr"], 0x0303_0509u32);
        assert_eq!(snap["pll1fracr"], 0x0000_8000u32);
        assert_eq!(snap["icscr"], 0x4400_0000u32, "ICSCR1 reset is state");
        assert_eq!(snap["icscr2"], 0x0000_001Fu32);
        assert_eq!(snap["icscr3"], 0x001F_0000u32);
        assert_eq!(snap["ahb2enr2"], 0x0000_0004u32);
        assert_eq!(snap["ahb3enr"], 0x0000_0008u32);

        // The WBA layout raises the flag, but it is still not serialized.
        let wba = Rcc::new_with_layout(RccRegisterLayout::Stm32Wba);
        assert!(wba.snapshot().get("synthesize_wba_rclk_pre_rdy").is_none());
        assert!(wba.snapshot().get("u5_cr_ready").is_none());
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
        // PLL1DIVR/PLL1FRACR are what HAL_RCC_GetSysClockFreq reads back after
        // OscConfig. Reset matches the constant HAL_RCC_DeInit writes.
        assert_eq!(rcc.read_u32(0x34).unwrap(), 0x0101_0280, "PLL1DIVR reset");
        assert_eq!(rcc.read_u32(0x38).unwrap(), 0, "PLL1FRACR reset");
        assert_eq!(rcc.read_u32(0x3C).unwrap(), 0x0101_0280, "PLL2DIVR reset");
        assert_eq!(rcc.read_u32(0x44).unwrap(), 0x0101_0280, "PLL3DIVR reset");
        rcc.write_u32(0x34, 0x0200_31F9).unwrap();
        rcc.write_u32(0x38, 0x0000_2000).unwrap();
        assert_eq!(rcc.read_u32(0x34).unwrap(), 0x0200_31F9);
        assert_eq!(rcc.read_u32(0x38).unwrap(), 0x0000_2000);
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

    /// STM32WB (RM0434 §6.4) puts the enable registers in the L4-shaped block —
    /// AHB2ENR@0x4C, APB1ENR1@0x58, APB2ENR@0x60 — NOT the H5-style V2 slots.
    /// Every number below is read off `tests/fixtures/real_world/stm32wb55.svd`
    /// and ST's CMSIS `stm32wb55xx.h`, never off the model.
    ///
    /// This part shipped on the H5-style layout, which meant the clock-enable
    /// write every STM32Cube / Zephyr / Arduino WB55 build performs went to an
    /// offset the model did not decode and was dropped on the floor, leaving
    /// TIM1/TIM2/I2C1/SPI1/ADC1/RTC permanently gated off. It survived because
    /// WB55's GPIO and both UARTs are ungated, so blinky and every UART-trace
    /// validation passed regardless.
    #[test]
    fn test_rcc_wb_offsets_match_rm0434() {
        let mut rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32Wb);

        // The gate resolver and the register decode must agree, and both must
        // agree with the SVD.
        assert_eq!(rcc.rcc_reg_offset("ahb2enr"), Some(0x4C));
        assert_eq!(rcc.rcc_reg_offset("apb1enr"), Some(0x58));
        assert_eq!(rcc.rcc_reg_offset("apb1enr1"), Some(0x58));
        assert_eq!(rcc.rcc_reg_offset("apb2enr"), Some(0x60));

        for (off, val) in [
            (0x4Cu64, 0x0002_00F0u32),
            (0x58, 0x0000_2001),
            (0x60, 0x0000_1800),
        ] {
            rcc.write_u32(off, val).unwrap();
            assert_eq!(
                rcc.read_u32(off).unwrap(),
                val,
                "RM0434 enable register @ {off:#X} must latch"
            );
        }

        // 0x9C is RCC_HSECR on this part — a real register with unrelated
        // semantics. It must NOT behave as APB1ENR1: writing it must not
        // disturb the enable register the clock gate actually reads.
        rcc.write_u32(0x9C, 0xFFFF_FFFF).unwrap();
        assert_eq!(
            rcc.read_u32(0x58).unwrap(),
            0x0000_2001,
            "a write to HSECR@0x9C must not touch APB1ENR1@0x58"
        );

        // And the H5-style slots must not answer as enable registers here.
        assert_eq!(rcc.read_u32(0x8C).unwrap(), 0x00);
        assert_eq!(rcc.read_u32(0xA4).unwrap(), 0x00);
    }

    /// U5 (`stm32v2`) keeps the H5/WBA enable/reset placement for the shared
    /// slots — AHB2ENR1@0x8C, APB1ENR1@0x9C, APB2ENR@0xA4 — so the WB fix does
    /// not leak into it. It also resolves its own clock-source names (CR@0x00,
    /// CRRCR@0x14, NOT the classic 0x98) and the full U5-only block:
    /// AHB1ENR@0x88, AHB2ENR2@0x90, AHB3ENR@0x94, APB1ENR2@0xA0,
    /// APB3ENR@0xA8 plus the eight RSTR registers and BDCR/CSR. Every offset
    /// from the vendored `configs/peripherals/stm32u575/rcc.yaml`.
    #[test]
    fn test_rcc_v2_h5_placement_unchanged() {
        let rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32V2);
        assert_eq!(rcc.rcc_reg_offset("ahb2enr"), Some(0x8C));
        assert_eq!(rcc.rcc_reg_offset("apb1enr"), Some(0x9C));
        assert_eq!(rcc.rcc_reg_offset("apb2enr"), Some(0xA4));
        assert_eq!(rcc.rcc_reg_offset("cr"), Some(0x00));
        assert_eq!(rcc.rcc_reg_offset("crrcr"), Some(0x14), "U5 CRRCR");
        assert_eq!(rcc.rcc_reg_offset("cfgr1"), Some(0x1C), "U5 CFGR1");
        for (name, off) in [
            ("ahb1enr", 0x88u64),
            ("ahb2enr1", 0x8C),
            ("ahb2enr2", 0x90),
            ("ahb3enr", 0x94),
            ("apb1enr1", 0x9C),
            ("apb1enr2", 0xA0),
            ("apb3enr", 0xA8),
            ("ahb1rstr", 0x60),
            ("ahb2rstr", 0x64),
            ("ahb2rstr2", 0x68),
            ("ahb3rstr", 0x6C),
            ("apb1rstr", 0x74),
            ("apb1rstr2", 0x78),
            ("apb2rstr", 0x7C),
            ("apb3rstr", 0x80),
            ("bdcr", 0xF0),
            ("csr", 0xF4),
        ] {
            assert_eq!(rcc.rcc_reg_offset(name), Some(off), "U5 {name}");
        }

        // The WB placement still resolves through the same map on its instance.
        let wb = Rcc::new_with_layout(RccRegisterLayout::Stm32Wb);
        assert_eq!(wb.rcc_reg_offset("ahb2enr"), Some(0x4C));
        assert_eq!(wb.rcc_reg_offset("crrcr"), Some(0x98), "WB CRRCR");
        // WB does not have the U5-only block names.
        assert_eq!(wb.rcc_reg_offset("ahb1enr"), None);
        assert_eq!(wb.rcc_reg_offset("apb3enr"), None);
        assert_eq!(wb.rcc_reg_offset("bdcr"), None);
    }

    /// G4 (RM0440) puts the enable registers at the L4 offsets — APB1ENR1@0x58,
    /// AHB2ENR@0x4C, APB2ENR@0x60 — NOT the H5-style V2 slots (0x9C/0x8C/0xA4).
    /// The clock-gate check resolves `clock: { reg }` through `rcc_reg_offset`,
    /// so a wrong offset here silently ungates every I2C1/TIM2 access.
    #[test]
    fn test_rcc_g4_offsets() {
        let rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32G4);
        assert_eq!(
            rcc.rcc_reg_offset("apb1enr"),
            Some(0x58),
            "APB1ENR1 (I2C1EN/TIM2EN)"
        );
        assert_eq!(rcc.rcc_reg_offset("apb1enr1"), Some(0x58));
        assert_eq!(rcc.rcc_reg_offset("ahb2enr"), Some(0x4C), "AHB2ENR (ADC12)");
        assert_eq!(
            rcc.rcc_reg_offset("apb2enr"),
            Some(0x60),
            "APB2ENR (TIM1/SPI1)"
        );
        assert_eq!(rcc.rcc_reg_offset("ahb1enr"), Some(0x48));

        // The V2 (H5-style) slots must NOT be the enable registers on G4 — a
        // write there is inert storage, proving the two layouts stay apart.
        let mut rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32G4);
        rcc.write_u32(0x58, 1 << 21).unwrap(); // APB1ENR1.I2C1EN
        assert_eq!(
            rcc.read_u32(0x58).unwrap(),
            1 << 21,
            "I2C1EN round-trips at 0x58"
        );
        rcc.write_u32(0x4C, 0xF0).unwrap();
        rcc.write_u32(0x60, 0x1800).unwrap();
        assert_eq!(rcc.read_u32(0x4C).unwrap(), 0xF0);
        assert_eq!(rcc.read_u32(0x60).unwrap(), 0x1800);
        assert_eq!(
            rcc.read_u32(0x9C).unwrap(),
            0x00,
            "0x9C is not an ENR on G4"
        );
    }

    /// G4 shares the V2 kernel-clock ready rules the family boots on: HSI16RDY at
    /// CR bit 10, HSI48RDY via CRRCR (0x98), LSI via CSR (0x94).
    #[test]
    fn test_rcc_g4_clock_ready() {
        let mut rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32G4);
        let cr = rcc.read_u32(0x00).unwrap();
        rcc.write_u32(0x00, cr | (1 << 8)).unwrap(); // HSION
        assert_ne!(
            rcc.read_u32(0x00).unwrap() & (1 << 10),
            0,
            "HSI16RDY at bit 10"
        );
        rcc.write_u32(0x98, 1).unwrap(); // HSI48ON
        assert_eq!(rcc.read_u32(0x98).unwrap() & 0x3, 0x3, "HSI48RDY");
        rcc.write_u32(0x94, 1).unwrap(); // LSION
        assert_eq!(rcc.read_u32(0x94).unwrap() & 0x3, 0x3, "LSIRDY");
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

    /// STM32G0 (RM0444 §5) has its own register map — verified against ST's
    /// CMSIS `stm32g071xx.h` RCC_TypeDef, never off the model. The load-bearing
    /// numbers: CR reset 0x0000_0500 (HSION bit8|HSIRDY bit10), IOPENR@0x34,
    /// AHBENR@0x38, APBENR1@0x3C (USART2EN bit17), APBENR2@0x40
    /// (USART1EN bit14), IOPRSTR@0x24, BDCR@0x5C, CSR@0x60. CFGR SW/SWS are
    /// 3-bit (SW bits 0-2, SWS bits 3-5) with HSISYS=000, HSE=001, PLLRCLK=010,
    /// LSI=011, LSE=100.
    ///
    /// This is deliberately NOT the L0 map: L0's IOPENR is at 0x2C and its
    /// CR-ready pair is HSI16 bit0→bit2. Sharing the L0 model would put the
    /// GPIO clock-enable write on a register the G0 RCC does not have there,
    /// leaving every GPIO/clock-gated peripheral dead on silicon.
    #[test]
    fn test_rcc_g0_layout_and_clock_switch() {
        let mut rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32G0);

        // Reset: HSION|HSIRDY = 0x0000_0500 (RM0444 §5.4.1). Bits 0-7 are
        // reserved on G0, so byte 0 reads 0 and byte 1 reads 0b101.
        assert_eq!(rcc.read_u32(0x00).unwrap(), 0x0000_0500, "G0 CR reset");
        assert_eq!(rcc.read(0x00).unwrap(), 0x00);
        assert_eq!(rcc.read(0x01).unwrap(), 0x05);

        // Gate/offset resolver must agree with the decode.
        assert_eq!(rcc.rcc_reg_offset("iopenr"), Some(0x34));
        assert_eq!(rcc.rcc_reg_offset("ahbenr"), Some(0x38));
        assert_eq!(rcc.rcc_reg_offset("apbenr1"), Some(0x3C));
        assert_eq!(rcc.rcc_reg_offset("apb1enr"), Some(0x3C));
        assert_eq!(rcc.rcc_reg_offset("apbenr2"), Some(0x40));
        assert_eq!(rcc.rcc_reg_offset("apb2enr"), Some(0x40));
        assert_eq!(rcc.rcc_reg_offset("cr"), Some(0x00));
        assert_eq!(rcc.rcc_reg_offset("bdcr"), Some(0x5C));
        assert_eq!(rcc.rcc_reg_offset("csr"), Some(0x60));

        // The L0 offsets must NOT answer on G0 (0x2C is APBRSTR1 here; 0x30 is
        // APBRSTR2; 0x08 is CFGR, not CRRCR).
        assert_eq!(rcc.read_u32(0x08).unwrap(), 0x00, "CFGR reset, not CRRCR");

        // Enabling HSI16 is a no-op ready-wise at reset; HSEON latches
        // HSERDY, PLLON latches PLLRDY (CR 16→17, 24→25).
        rcc.write_u32(0x00, 0x500 | (1 << 16) | (1 << 24)).unwrap();
        let cr = rcc.read_u32(0x00).unwrap();
        assert_ne!(cr & (1 << 17), 0, "HSERDY follows HSEON");
        assert_ne!(cr & (1 << 25), 0, "PLLRDY follows PLLON");

        // SYSCLK switch: SW=010 (PLLRCLK) with PLL ready completes and SWS
        // (bits 5:3) follows; then clearing PLLON and asking again holds SWS.
        rcc.write_u32(0x08, 0b010).unwrap();
        assert_eq!(
            (rcc.read_u32(0x08).unwrap() >> 3) & 0x7,
            0b010,
            "SWS=PLLRCLK once PLLRDY"
        );
        rcc.write_u32(0x00, 0x500).unwrap(); // PLLON=0
        rcc.write_u32(0x08, 0b100).unwrap(); // SW=LSE, LSE off
        assert_eq!(
            (rcc.read_u32(0x08).unwrap() >> 3) & 0x7,
            0b010,
            "SWS holds while the requested source is not ready"
        );
        // Bring LSE up (BDCR LSEON→LSERDY) and the same switch completes.
        rcc.write_u32(0x5C, 1).unwrap();
        rcc.write_u32(0x08, 0b100).unwrap();
        assert_eq!(
            (rcc.read_u32(0x08).unwrap() >> 3) & 0x7,
            0b100,
            "SWS=LSE once LSERDY"
        );

        // Enable registers round-trip at the G0 offsets: GPIOAEN in IOPENR,
        // USART2EN (bit17) in APBENR1, USART1EN (bit14) in APBENR2.
        rcc.write_u32(0x34, 1 << 0).unwrap();
        rcc.write_u32(0x3C, 1 << 17).unwrap();
        rcc.write_u32(0x40, 1 << 14).unwrap();
        assert_eq!(rcc.read_u32(0x34).unwrap(), 1 << 0, "IOPENR.GPIOAEN");
        assert_eq!(rcc.read_u32(0x3C).unwrap(), 1 << 17, "APBENR1.USART2EN");
        assert_eq!(rcc.read_u32(0x40).unwrap(), 1 << 14, "APBENR2.USART1EN");
        // The L0 IOPENR slot is APBRSTR1 on G0 and must stay untouched.
        assert_eq!(rcc.read_u32(0x2C).unwrap(), 0x00);

        // LSI (CSR @0x60) and LSE (BDCR @0x5C) ack their ready bits.
        rcc.write_u32(0x60, 1).unwrap();
        assert_eq!(rcc.read_u32(0x60).unwrap() & 0x3, 0x3, "G0 LSIRDY");
        assert_eq!(rcc.read_u32(0x5C).unwrap() & 0x3, 0x3, "G0 LSERDY");
    }

    /// The F030 clock bring-up STM32Cube and generated programs run (RM0360
    /// §6.2.3 / §6.4): PLLSRC = HSI/2, PLLMUL ×12, PLLON, wait PLLRDY, SW =
    /// PLL, wait SWS = PLL. PLLRDY follows PLLON only, and SWS follows SW only
    /// once the PLL is ready — never before.
    #[test]
    fn f0_pll_bring_up_ready_bits_follow_their_enables() {
        let mut rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32F0);
        assert_eq!(
            rcc.read_u32(0x00).unwrap() & 0x3,
            0x3,
            "HSION|HSIRDY at reset"
        );
        assert_eq!(
            rcc.read_u32(0x14).unwrap(),
            0x14,
            "AHBENR reset SRAMEN|FLITFEN"
        );

        // SW = PLL before the PLL runs: SWS must stay HSI.
        rcc.write_u32(0x04, (10 << 18) | 0b10).unwrap();
        assert_eq!((rcc.read_u32(0x04).unwrap() >> 2) & 3, 0b00, "no PLL yet");
        assert_eq!(rcc.read_u32(0x00).unwrap() & (1 << 25), 0, "PLLRDY off");

        rcc.write_u32(0x00, rcc.read_u32(0x00).unwrap() | (1 << 24))
            .unwrap();
        assert_ne!(
            rcc.read_u32(0x00).unwrap() & (1 << 25),
            0,
            "PLLRDY follows PLLON"
        );
        rcc.write_u32(0x04, (10 << 18) | 0b10).unwrap();
        assert_eq!((rcc.read_u32(0x04).unwrap() >> 2) & 3, 0b10, "SWS = PLL");
        assert_eq!((rcc.read_u32(0x04).unwrap() >> 18) & 0xF, 10, "PLLMUL kept");

        // PLL off again: PLLRDY drops.
        rcc.write_u32(0x00, rcc.read_u32(0x00).unwrap() & !(1 << 24))
            .unwrap();
        assert_eq!(rcc.read_u32(0x00).unwrap() & (1 << 25), 0, "PLLRDY drops");

        // F0 GPIO clocks live in AHBENR bits 17..22 and read back.
        rcc.write_u32(0x14, 0x14 | (1 << 17) | (1 << 19)).unwrap();
        assert_eq!(rcc.read_u32(0x14).unwrap(), 0x14 | (1 << 17) | (1 << 19));

        // CR2: HSI14ON → HSI14RDY (the ADC oscillator), off at reset.
        assert_eq!(rcc.read_u32(0x34).unwrap() & 0x3, 0, "HSI14 off at reset");
        rcc.write_u32(0x34, 0x80 | 1).unwrap();
        assert_eq!(
            rcc.read_u32(0x34).unwrap() & 0x3,
            0x3,
            "HSI14RDY follows HSI14ON"
        );

        assert_eq!(
            "stm32f0".parse::<RccRegisterLayout>().unwrap(),
            RccRegisterLayout::Stm32F0
        );
        assert_eq!(rcc.rcc_reg_offset("ahbenr"), Some(0x14));
        assert_eq!(rcc.rcc_reg_offset("cr2"), Some(0x34));
    }

    /// G0's CR ready pair is at bit 8→10 (HSI16 style), which is why the L0
    /// model cannot serve it: L0's HSI16 is bit 0→2, and its MSI pair is
    /// 8→9. If G0 shared either, HSIRDY would read a different bit and every
    /// HAL/Zephyr bring-up polling HSIRDY would time out.
    #[test]
    fn test_rcc_g0_hsi_ready_bit_positions() {
        let mut rcc = Rcc::new_with_layout(RccRegisterLayout::Stm32G0);
        // HSI is on at reset; clear then re-enable and require bit10.
        rcc.write_u32(0x00, 0x0000_0500 & !(1 << 8)).unwrap();
        assert_eq!(
            rcc.read_u32(0x00).unwrap() & (1 << 10),
            0,
            "HSIRDY drops with HSION"
        );
        rcc.write_u32(0x00, 1 << 8).unwrap();
        assert_ne!(
            rcc.read_u32(0x00).unwrap() & (1 << 10),
            0,
            "HSIRDY at bit 10, not L0's bit 2"
        );
        assert_eq!(
            rcc.read_u32(0x00).unwrap() & (1 << 9),
            0,
            "bit 9 is HSIKERON (not a ready flag)"
        );

        // The layout parses from the chip yaml `profile` selector.
        assert_eq!(
            "stm32g0".parse::<RccRegisterLayout>().unwrap(),
            RccRegisterLayout::Stm32G0
        );
    }
}
