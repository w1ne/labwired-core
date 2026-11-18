// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! UF2 container loading (Microsoft UF2 specification, github.com/microsoft/uf2,
//! README "File format"): 512-byte blocks, each carrying up to 476 payload
//! bytes for one target address.
//!
//! Block layout (little-endian words): magicStart0 `0x0A324655`, magicStart1
//! `0x9E5D5157`, flags, targetAddr, payloadSize, blockNo, numBlocks,
//! fileSize-or-familyID, data[476], magicEnd `0x0AB16F30`.
//!
//! Flags honoured: `0x0000_0001` not-main-flash (the block is skipped, as the
//! spec directs), `0x0000_2000` familyID present, `0x0000_4000` MD5 checksum
//! present and `0x0000_8000` extension tags present (both carried inside the
//! block and ignored here). `0x0000_1000` file container is refused: it names
//! files, not memory.
//!
//! The family selects the architecture. Only families whose images LabWired
//! executes are mapped (pico-sdk `boot/uf2.h`): RP2040 and RP2350 Arm
//! (Secure and Non-secure), plus the RP2350 `absolute` and `data` families,
//! which carry bytes for a block whose image family is given elsewhere in the
//! file. Every other family, a file without any family, and RISC-V images are
//! refused with the reason.

use anyhow::{anyhow, bail, Result};
use labwired_core::memory::ProgramImage;
use labwired_core::Arch;

const MAGIC_START0: u32 = 0x0A32_4655;
const MAGIC_START1: u32 = 0x9E5D_5157;
const MAGIC_END: u32 = 0x0AB1_6F30;
const BLOCK: usize = 512;
const MAX_PAYLOAD: u32 = 476;

const FLAG_NOT_MAIN_FLASH: u32 = 0x0000_0001;
const FLAG_FILE_CONTAINER: u32 = 0x0000_1000;
const FLAG_FAMILY_ID: u32 = 0x0000_2000;

pub const FAMILY_RP2040: u32 = 0xE48B_FF56;
pub const FAMILY_ABSOLUTE: u32 = 0xE48B_FF57;
pub const FAMILY_DATA: u32 = 0xE48B_FF58;
pub const FAMILY_RP2350_ARM_S: u32 = 0xE48B_FF59;
pub const FAMILY_RP2350_RISCV: u32 = 0xE48B_FF5A;
pub const FAMILY_RP2350_ARM_NS: u32 = 0xE48B_FF5B;

/// True when `bytes` starts with a UF2 block header.
pub fn is_uf2(bytes: &[u8]) -> bool {
    bytes.len() >= 8 && word(bytes, 0) == MAGIC_START0 && word(bytes, 4) == MAGIC_START1
}

fn word(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

/// Parse a UF2 file into a [`ProgramImage`]: one segment per run of
/// contiguous payload, architecture from the family ID. The entry point is 0:
/// a UF2 carries none, so the core resets the way the chip does (vector table
/// or boot ROM).
pub fn load_uf2_bytes(bytes: &[u8]) -> Result<ProgramImage> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(BLOCK) {
        bail!(
            "UF2: {} bytes is not a whole number of 512-byte blocks",
            bytes.len()
        );
    }
    let mut image_family: Option<u32> = None;
    let mut chunks: Vec<(u64, &[u8])> = Vec::new();
    for (n, b) in bytes.chunks_exact(BLOCK).enumerate() {
        if word(b, 0) != MAGIC_START0 || word(b, 4) != MAGIC_START1 || word(b, 508) != MAGIC_END {
            bail!("UF2 block {n}: bad magic");
        }
        let flags = word(b, 8);
        let addr = word(b, 12);
        let size = word(b, 16);
        if flags & FLAG_FILE_CONTAINER != 0 {
            bail!("UF2 block {n}: file-container blocks name files, not memory; not loadable");
        }
        if flags & FLAG_FAMILY_ID == 0 {
            bail!("UF2 block {n}: no family ID, so the target architecture is unknown");
        }
        if size > MAX_PAYLOAD {
            bail!("UF2 block {n}: payload size {size} exceeds {MAX_PAYLOAD}");
        }
        if flags & FLAG_NOT_MAIN_FLASH != 0 {
            continue;
        }
        let family = word(b, 28);
        match family {
            FAMILY_RP2040 | FAMILY_RP2350_ARM_S | FAMILY_RP2350_ARM_NS => {
                if let Some(f) = image_family.filter(|f| *f != family) {
                    bail!("UF2 block {n}: family {family:#010x} mixed with {f:#010x}");
                }
                image_family = Some(family);
            }
            FAMILY_ABSOLUTE | FAMILY_DATA => {}
            FAMILY_RP2350_RISCV => {
                bail!("UF2 block {n}: RP2350 RISC-V (Hazard3) images are not modelled")
            }
            other => bail!("UF2 block {n}: family {other:#010x} is not mapped to an architecture"),
        }
        chunks.push((u64::from(addr), &b[32..32 + size as usize]));
    }
    let family = image_family.ok_or_else(|| {
        anyhow!("UF2: no block names an executable family (only absolute/data blocks)")
    })?;
    let arch = match family {
        FAMILY_RP2040 | FAMILY_RP2350_ARM_S | FAMILY_RP2350_ARM_NS => Arch::Arm,
        _ => unreachable!(),
    };
    chunks.sort_by_key(|(a, _)| *a);
    let mut image = ProgramImage::new(0, arch);
    let mut run: Option<(u64, Vec<u8>)> = None;
    for (addr, data) in chunks {
        match run.as_mut() {
            Some((start, buf)) if *start + buf.len() as u64 == addr => buf.extend_from_slice(data),
            Some((start, buf)) if *start + (buf.len() as u64) > addr => {
                bail!("UF2: block at {addr:#010x} overlaps the run starting at {start:#010x}")
            }
            _ => {
                if let Some((start, buf)) = run.take() {
                    image.add_segment(start, buf);
                }
                run = Some((addr, data.to_vec()));
            }
        }
    }
    if let Some((start, buf)) = run {
        image.add_segment(start, buf);
    }
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(flags: u32, addr: u32, data: &[u8], no: u32, total: u32, family: u32) -> Vec<u8> {
        let mut b = Vec::with_capacity(BLOCK);
        for w in [
            MAGIC_START0,
            MAGIC_START1,
            flags,
            addr,
            data.len() as u32,
            no,
            total,
            family,
        ] {
            b.extend_from_slice(&w.to_le_bytes());
        }
        b.extend_from_slice(data);
        b.resize(508, 0);
        b.extend_from_slice(&MAGIC_END.to_le_bytes());
        b
    }

    #[test]
    fn contiguous_blocks_form_one_segment_with_the_family_arch() {
        let mut f = block(
            FLAG_FAMILY_ID,
            0x1000_0100,
            &[2; 256],
            1,
            2,
            FAMILY_RP2350_ARM_S,
        );
        f.extend(block(
            FLAG_FAMILY_ID,
            0x1000_0000,
            &[1; 256],
            0,
            2,
            FAMILY_RP2350_ARM_S,
        ));
        let img = load_uf2_bytes(&f).unwrap();
        assert_eq!(img.arch, Arch::Arm);
        assert_eq!(img.segments.len(), 1);
        assert_eq!(img.segments[0].start_addr, 0x1000_0000);
        assert_eq!(img.segments[0].data.len(), 512);
        assert_eq!(img.segments[0].data[0], 1);
        assert_eq!(img.segments[0].data[256], 2);
    }

    #[test]
    fn gaps_split_segments_and_not_main_flash_blocks_are_skipped() {
        let mut f = block(FLAG_FAMILY_ID, 0x1000_0000, &[1; 256], 0, 3, FAMILY_RP2040);
        f.extend(block(
            FLAG_FAMILY_ID,
            0x1000_1000,
            &[3; 256],
            1,
            3,
            FAMILY_RP2040,
        ));
        f.extend(block(
            FLAG_FAMILY_ID | FLAG_NOT_MAIN_FLASH,
            0x2000_0000,
            &[9; 16],
            2,
            3,
            FAMILY_RP2040,
        ));
        let img = load_uf2_bytes(&f).unwrap();
        let starts: Vec<u64> = img.segments.iter().map(|s| s.start_addr).collect();
        assert_eq!(starts, vec![0x1000_0000, 0x1000_1000]);
    }

    #[test]
    fn refuses_unmapped_families_containers_and_bad_magic() {
        let riscv = block(
            FLAG_FAMILY_ID,
            0x1000_0000,
            &[0; 4],
            0,
            1,
            FAMILY_RP2350_RISCV,
        );
        assert!(load_uf2_bytes(&riscv)
            .unwrap_err()
            .to_string()
            .contains("RISC-V"));
        let other = block(FLAG_FAMILY_ID, 0, &[0; 4], 0, 1, 0xADA5_2840);
        assert!(load_uf2_bytes(&other)
            .unwrap_err()
            .to_string()
            .contains("not mapped"));
        let no_family = block(0, 0, &[0; 4], 0, 1, 1234);
        assert!(load_uf2_bytes(&no_family)
            .unwrap_err()
            .to_string()
            .contains("no family"));
        let container = block(
            FLAG_FAMILY_ID | FLAG_FILE_CONTAINER,
            0,
            &[0; 4],
            0,
            1,
            FAMILY_RP2040,
        );
        assert!(load_uf2_bytes(&container)
            .unwrap_err()
            .to_string()
            .contains("file-container"));
        let mut bad = block(FLAG_FAMILY_ID, 0, &[0; 4], 0, 1, FAMILY_RP2040);
        bad[508] ^= 1;
        assert!(load_uf2_bytes(&bad)
            .unwrap_err()
            .to_string()
            .contains("magic"));
        let data_only = block(FLAG_FAMILY_ID, 0x1000_0000, &[0; 4], 0, 1, FAMILY_DATA);
        assert!(load_uf2_bytes(&data_only)
            .unwrap_err()
            .to_string()
            .contains("executable family"));
    }

    #[test]
    fn detects_the_header() {
        let f = block(FLAG_FAMILY_ID, 0x1000_0000, &[0; 4], 0, 1, FAMILY_RP2040);
        assert!(is_uf2(&f));
        assert!(!is_uf2(b"\x7fELF...."));
    }
}
