// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Pulls configured outside the GPIO block.
//!
//! Kinetis, Renesas RA and i.MX RT keep a pad's pull resistor in a pad-control
//! block, not in the GPIO port: `PORTx_PCRn` (PE/PS), `PmnPFS` (PCR) and the
//! IOMUXC `SW_PAD_CTL_PAD_*` registers (PKE/PUE/PUS). The GPIO port still
//! owns the pad's [`PinPort`](super::PinPort), so the pull has to reach it:
//! the port names its pad-control block ([`PinPort::pad_control`](super::PinPort::pad_control)),
//! and after every write to that block the bus decodes each pad's pull from
//! the block's registers with [`PadControl::pull`] and hands it to the port
//! ([`PinPort::set_config_pull`](super::PinPort::set_config_pull)),
//! bracketed for push capture like any other pad mutation. The port then
//! reports it in `driver().pull` and folds it into its input register, as for
//! a pull the port keeps itself.

use super::Pull;
use crate::Peripheral;

/// How a pad-control block encodes the pull of one GPIO port's pads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PadControl {
    /// NXP Kinetis `PORTx_PCRn` at `4·n` (KW41Z RM, PORT chapter): PE (bit 1)
    /// enables the pull, PS (bit 0) picks up (1) or down (0). The pull acts
    /// on a digital pin only, so MUX (bits 10:8) = 0, the pin disabled /
    /// analog, has none.
    KinetisPcr,
    /// Renesas RA `PmnPFS` at `0x40·m + 4·n` from the PFS base (RA4M1 UM
    /// R01UH0887 §19.2.5): PCR (bit 4) is the input pull-up. RA has no
    /// pull-down.
    RaPfs {
        /// The port number `m` this GPIO port is.
        port: u8,
    },
    /// NXP i.MX RT IOMUXC `SW_PAD_CTL_PAD_*` at `base + 4·n` (IMXRT1060RM,
    /// IOMUXC chapter): a pull acts while PKE (bit 12) and PUE (bit 13) are both set
    /// (PUE clear selects the keeper, which is no pull); PUS (bits 15:14)
    /// 00 is the 100 kΩ pull-down, 01/10/11 the 47k/100k/22k pull-ups.
    /// `base` is the pad-control register of the port's pin 0; the port's
    /// pads must be contiguous in IOMUXC from there (GPIO1 `GPIO_AD_B0_00`,
    /// GPIO2 `GPIO_B0_00`, GPIO4 `GPIO_EMC_00` are; GPIO3 is not).
    ImxrtPadCtl {
        /// Offset of pin 0's `SW_PAD_CTL_PAD` register in the IOMUXC window.
        base: u64,
    },
}

impl PadControl {
    /// Offset of the register holding `pin`'s pull in the pad-control block.
    pub fn offset(self, pin: u8) -> u64 {
        let pin = u64::from(pin);
        match self {
            Self::KinetisPcr => 4 * pin,
            Self::RaPfs { port } => 0x40 * u64::from(port) + 4 * pin,
            Self::ImxrtPadCtl { base } => base + 4 * pin,
        }
    }

    /// The pull one register word configures.
    pub fn decode(self, word: u32) -> Pull {
        match self {
            Self::KinetisPcr => {
                if (word >> 8) & 0x7 == 0 || word & 0b10 == 0 {
                    Pull::None
                } else if word & 0b01 != 0 {
                    Pull::Up
                } else {
                    Pull::Down
                }
            }
            Self::RaPfs { .. } => {
                if word & (1 << 4) != 0 {
                    Pull::Up
                } else {
                    Pull::None
                }
            }
            Self::ImxrtPadCtl { .. } => {
                let pke = word & (1 << 12) != 0;
                let pue = word & (1 << 13) != 0;
                if !(pke && pue) {
                    Pull::None
                } else if (word >> 14) & 0x3 == 0 {
                    Pull::Down
                } else {
                    Pull::Up
                }
            }
        }
    }

    /// The pin whose output flip-flops a `width`-byte store at `offset` into
    /// the block writes, for blocks that share them with the GPIO port.
    ///
    /// Renesas RA only: `PmnPFS` PODR (bit 0) and PDR (bit 2) are the same
    /// flip-flops as the port's `PCNTR1` PODR/PDR (R01UH0887 §19.2.5), so a
    /// store that covers bits 2:0 of this port's `PmnPFS` word (the word,
    /// its low half, or `PmnPFS_HA` / `PmnPFS_BY`, which alias the low half
    /// and byte) sets them. A store to the upper bytes does not.
    pub fn output_write(self, offset: u64, width: u64) -> Option<u8> {
        let Self::RaPfs { port } = self else {
            return None;
        };
        let word = offset & !3;
        if word / 0x40 != u64::from(port) {
            return None;
        }
        match (offset % 4, width) {
            (0, _) | (2, 2) | (3, 1) => Some(((word % 0x40) / 4) as u8),
            _ => None,
        }
    }

    /// The direction and output level one register word sets: `(PDR, PODR)`.
    /// Only meaningful for a pin [`Self::output_write`] named.
    pub fn decode_output(self, word: u32) -> (bool, bool) {
        (word & (1 << 2) != 0, word & 1 != 0)
    }

    /// The pull `block` (the pad-control peripheral) configures on `pin`. A
    /// register the block cannot read is no pull.
    pub fn pull(self, block: &dyn Peripheral, pin: u8) -> Pull {
        block
            .read_u32(self.offset(pin))
            .map(|word| self.decode(word))
            .unwrap_or(Pull::None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinetis_pcr_needs_pe_and_a_digital_mux() {
        let pcr = PadControl::KinetisPcr;
        assert_eq!(pcr.offset(5), 0x14);
        assert_eq!(pcr.decode(0x0000_0103), Pull::Up, "MUX=1 PE PS");
        assert_eq!(pcr.decode(0x0000_0102), Pull::Down, "MUX=1 PE");
        assert_eq!(pcr.decode(0x0000_0101), Pull::None, "PS without PE");
        assert_eq!(pcr.decode(0x0000_0003), Pull::None, "MUX=0: disabled pin");
        assert_eq!(pcr.decode(0x0000_0005), Pull::None, "KW41Z reset value");
    }

    #[test]
    fn ra_pfs_pcr_is_a_pull_up_only() {
        let pfs = PadControl::RaPfs { port: 1 };
        assert_eq!(pfs.offset(11), 0x40 + 0x2C);
        assert_eq!(pfs.decode(1 << 4), Pull::Up);
        assert_eq!(pfs.decode(1 << 2), Pull::None, "PDR alone");
    }

    #[test]
    fn ra_pfs_low_byte_stores_write_the_port_output() {
        let pfs = PadControl::RaPfs { port: 6 };
        // P600PFS word, its PmnPFS_HA (+2, 16-bit) and PmnPFS_BY (+3, 8-bit).
        assert_eq!(pfs.output_write(0x180, 4), Some(0));
        assert_eq!(pfs.output_write(0x180, 1), Some(0));
        assert_eq!(pfs.output_write(0x182, 2), Some(0));
        assert_eq!(pfs.output_write(0x183, 1), Some(0));
        assert_eq!(pfs.output_write(0x1BC, 4), Some(15), "P615PFS");
        // Upper bytes (PSEL/PMR/ISEL) do not reach PDR/PODR.
        assert_eq!(pfs.output_write(0x181, 1), None);
        assert_eq!(pfs.output_write(0x182, 1), None);
        // Another port's word, and PWPR, are not this port's pins.
        assert_eq!(pfs.output_write(0x140, 4), None);
        assert_eq!(pfs.output_write(0x50C, 1), None);
        assert_eq!(pfs.decode_output(0b101), (true, true));
        assert_eq!(pfs.decode_output(0b100), (true, false));
        assert_eq!(pfs.decode_output(0b001), (false, true));
        assert_eq!(PadControl::KinetisPcr.output_write(0, 4), None);
    }

    #[test]
    fn imxrt_pad_ctl_pull_needs_pke_and_pue() {
        let pad = PadControl::ImxrtPadCtl { base: 0x32C };
        assert_eq!(pad.offset(3), 0x338);
        assert_eq!(pad.decode(0x10B0), Pull::None, "reset: keeper");
        assert_eq!(pad.decode(0x3000), Pull::Down, "PUS 00: 100k down");
        assert_eq!(pad.decode(0x7000), Pull::Up, "PUS 01: 47k up");
        assert_eq!(pad.decode(0xF000), Pull::Up, "PUS 11: 22k up");
        assert_eq!(pad.decode(0xD000), Pull::None, "PUE without PKE");
    }
}
