// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Compose an ARM flash image out of several pieces placed at explicit,
//! user-supplied offsets — e.g. a Nordic SoftDevice at `0x0` plus an
//! application ELF that was linked to run above it.
//!
//! This is the general mechanism behind `labwired run --flash-image
//! <path>@<hex-offset>` (repeatable). It is deliberately NOT auto-detecting
//! anything: the caller states exactly where each piece goes, and overlapping
//! pieces are a hard error rather than silent last-writer-wins.
//!
//! Each piece may be:
//!   * an ELF (`PT_LOAD` program headers, physical addresses used as-is,
//!     then shifted by `offset`),
//!   * an Intel HEX (`.hex`) file, parsed directly (records + extended
//!     linear/segment address support), addresses shifted by `offset`,
//!   * a raw binary blob, placed starting at `offset`.
//!
//! Note: `objcopy -O ihex` output piped back through `objcopy -O elf32-*`
//! is NOT loadable by [`crate::load_elf_bytes`] — it round-trips through a
//! flat blob with no `PT_LOAD` headers. Either hand this module the `.hex`
//! directly, or convert with `objcopy -O binary` (not `-O elf32-*`) first.

use anyhow::{anyhow, bail, Context, Result};
use labwired_core::memory::Segment;
use std::path::{Path, PathBuf};

/// Extract only the real `SHF_ALLOC` sections with actual bytes (skips
/// `SHT_NOBITS` like `.bss`/`.heap`, and skips debug/symbol sections that
/// have no `ALLOC` flag) from an ELF, at their section-header addresses.
///
/// This is deliberately NOT the same thing as walking `PT_LOAD` program
/// headers (see [`crate::load_elf_bytes`]): some toolchains (observed with
/// the Adafruit nRF52 core, whose `.ld` scripts request 64KB `PT_LOAD`
/// alignment for DFU/OTA page-erase reasons) emit a `PT_LOAD` header whose
/// `p_vaddr`/`p_paddr` is rounded down to the alignment boundary while the
/// real code starts partway in, backed on disk by nothing more than the ELF
/// header and zero padding. Using `p_paddr` as the segment's flash address
/// in that case loads several KB of ELF-header garbage into a flash range a
/// real flashing tool (which works section-by-section) never touches — and
/// when that range is legitimately owned by another `--flash-image` piece
/// (e.g. a SoftDevice), it manifests as a false overlap.
///
/// Used for `--flash-image` composition (both the extra pieces and, by the
/// CLI, the primary `--firmware` ELF) so multi-image placement reflects what
/// actually ends up in flash on real hardware. The single-image
/// `labwired_loader::load_elf` path is untouched — this function is
/// additive, not a replacement for it.
pub fn elf_alloc_sections(buffer: &[u8]) -> Result<Vec<(u64, Vec<u8>)>> {
    use goblin::elf::section_header::{SHF_ALLOC, SHT_NOBITS};
    use goblin::elf::Elf;

    let elf = Elf::parse(buffer).context("failed to parse ELF binary")?;
    let mut out = Vec::new();
    for sh in &elf.section_headers {
        if sh.sh_flags as u32 & SHF_ALLOC == 0 {
            continue;
        }
        if sh.sh_type == SHT_NOBITS || sh.sh_size == 0 {
            continue; // .bss/.heap: no on-disk bytes, nothing to place in flash.
        }
        let start = sh.sh_offset as usize;
        let end = start + sh.sh_size as usize;
        if end > buffer.len() {
            bail!("section out of bounds in ELF file");
        }
        out.push((sh.sh_addr, buffer[start..end].to_vec()));
    }
    Ok(out)
}

/// One `--flash-image <path>@<hex-offset>` piece, resolved to concrete bytes.
#[derive(Debug, Clone)]
pub struct FlashImagePiece {
    pub path: PathBuf,
    pub offset: u64,
    pub segments: Vec<Segment>,
}

/// Parse a single `--flash-image` CLI argument of the form `<path>@<hex>`.
/// The offset is mandatory and always hex (with or without a `0x` prefix) —
/// this flag has no notion of "just append it wherever."
pub fn parse_flash_image_arg(arg: &str) -> Result<(PathBuf, u64)> {
    let (path_str, offset_str) = arg
        .rsplit_once('@')
        .ok_or_else(|| anyhow!("--flash-image {arg:?}: expected `<path>@<hex-offset>`"))?;
    if path_str.is_empty() {
        bail!("--flash-image {arg:?}: empty path before '@'");
    }
    let offset = u64::from_str_radix(
        offset_str.trim_start_matches("0x").trim_start_matches("0X"),
        16,
    )
    .with_context(|| format!("--flash-image {arg:?}: offset {offset_str:?} is not valid hex"))?;
    Ok((PathBuf::from(path_str), offset))
}

/// Load one flash-image piece: detect ELF / Intel-HEX / raw-binary by
/// content, extract its (address, bytes) segments, and shift every address
/// by `offset`.
pub fn load_flash_piece(path: &Path, offset: u64) -> Result<FlashImagePiece> {
    let buffer =
        std::fs::read(path).with_context(|| format!("failed to read --flash-image {path:?}"))?;

    let raw_segments: Vec<(u64, Vec<u8>)> =
        if buffer.len() >= 4 && buffer[0..4] == [0x7f, b'E', b'L', b'F'] {
            elf_alloc_sections(&buffer)
                .with_context(|| format!("--flash-image {path:?}: failed to parse as ELF"))?
        } else if buffer.first() == Some(&b':') {
            parse_ihex(&buffer)
                .with_context(|| format!("--flash-image {path:?}: failed to parse as Intel HEX"))?
        } else {
            // Raw binary: one segment, address 0 (relative to `offset`).
            vec![(0, buffer)]
        };

    let segments = raw_segments
        .into_iter()
        .filter(|(_, data)| !data.is_empty())
        .map(|(addr, data)| Segment {
            start_addr: addr + offset,
            data,
        })
        .collect();

    Ok(FlashImagePiece {
        path: path.to_path_buf(),
        offset,
        segments,
    })
}

/// Minimal Intel HEX parser: record types 00 (data), 01 (EOF), 02 (extended
/// segment address), 04 (extended linear address). Start-address records
/// (03/05) are ignored — flash-image pieces are placed at the caller's
/// `--flash-image` offset, not at whatever entry the hex file happens to
/// declare. Coalesces adjacent/overlapping data records into contiguous
/// segments so the overlap check downstream sees real byte ranges.
fn parse_ihex(buffer: &[u8]) -> Result<Vec<(u64, Vec<u8>)>> {
    let text = std::str::from_utf8(buffer).context("Intel HEX file is not valid UTF-8/ASCII")?;
    let mut upper_linear: u64 = 0;
    let mut upper_segment: u64 = 0;
    let mut chunks: Vec<(u64, Vec<u8>)> = Vec::new();

    for (lineno, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let line = line
            .strip_prefix(':')
            .ok_or_else(|| anyhow!("line {}: does not start with ':'", lineno + 1))?;
        let bytes =
            hex_decode(line).with_context(|| format!("line {}: not valid hex", lineno + 1))?;
        if bytes.len() < 5 {
            bail!("line {}: record too short", lineno + 1);
        }
        let byte_count = bytes[0] as usize;
        let addr = u16::from_be_bytes([bytes[1], bytes[2]]) as u64;
        let rec_type = bytes[3];
        if bytes.len() != 5 + byte_count {
            bail!("line {}: byte count mismatch", lineno + 1);
        }
        let data = &bytes[4..4 + byte_count];
        // (checksum at bytes[4+byte_count] is intentionally not verified —
        // files here come from a trusted local toolchain output directory.)
        match rec_type {
            0x00 => {
                let abs = upper_linear + upper_segment + addr;
                chunks.push((abs, data.to_vec()));
            }
            0x01 => break, // EOF
            0x02 => {
                if data.len() != 2 {
                    bail!(
                        "line {}: extended segment address record must be 2 bytes",
                        lineno + 1
                    );
                }
                upper_segment = (u16::from_be_bytes([data[0], data[1]]) as u64) << 4;
                upper_linear = 0;
            }
            0x04 => {
                if data.len() != 2 {
                    bail!(
                        "line {}: extended linear address record must be 2 bytes",
                        lineno + 1
                    );
                }
                upper_linear = (u16::from_be_bytes([data[0], data[1]]) as u64) << 16;
                upper_segment = 0;
            }
            0x03 | 0x05 => {} // start address records: ignored, see doc comment.
            other => bail!(
                "line {}: unsupported Intel HEX record type {other:#04x}",
                lineno + 1
            ),
        }
    }

    Ok(coalesce_chunks(chunks))
}

fn hex_decode(s: &str) -> Result<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        bail!("odd-length hex string");
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| anyhow!(e)))
        .collect()
}

/// Merge chunks that are exactly adjacent or overlapping-with-identical-data
/// into single contiguous runs, sorted by address. Kept deliberately simple:
/// ihex data records are almost always emitted in address order with a fixed
/// stride, so this just concatenates runs where `next.addr == prev.end`.
fn coalesce_chunks(mut chunks: Vec<(u64, Vec<u8>)>) -> Vec<(u64, Vec<u8>)> {
    chunks.sort_by_key(|(addr, _)| *addr);
    let mut out: Vec<(u64, Vec<u8>)> = Vec::new();
    for (addr, data) in chunks {
        if let Some((last_addr, last_data)) = out.last_mut() {
            let last_end = *last_addr + last_data.len() as u64;
            if addr == last_end {
                last_data.extend_from_slice(&data);
                continue;
            }
        }
        out.push((addr, data));
    }
    out
}

/// Check a set of segments (already at their final absolute addresses) for
/// any pairwise byte-range overlap. Returns a descriptive error naming both
/// offending ranges instead of silently letting the second writer win.
pub fn check_no_overlaps(segments: &[Segment]) -> Result<()> {
    let mut ranges: Vec<(u64, u64)> = segments
        .iter()
        .map(|s| (s.start_addr, s.start_addr + s.data.len() as u64))
        .collect();
    ranges.sort();
    for w in ranges.windows(2) {
        let (a_start, a_end) = w[0];
        let (b_start, b_end) = w[1];
        if b_start < a_end {
            bail!(
                "overlapping flash segments: [{a_start:#010x}, {a_end:#010x}) overlaps \
                 [{b_start:#010x}, {b_end:#010x})"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_offset_arg_accepts_0x_prefix() {
        let (path, off) = parse_flash_image_arg("softdevice.hex@0x1000").unwrap();
        assert_eq!(path, PathBuf::from("softdevice.hex"));
        assert_eq!(off, 0x1000);
    }

    #[test]
    fn parse_offset_arg_accepts_bare_hex() {
        let (_, off) = parse_flash_image_arg("app.bin@2fe9d").unwrap();
        assert_eq!(off, 0x2fe9d);
    }

    #[test]
    fn parse_offset_arg_rejects_missing_at() {
        assert!(parse_flash_image_arg("app.bin").is_err());
    }

    #[test]
    fn parse_offset_arg_rejects_bad_hex() {
        assert!(parse_flash_image_arg("app.bin@zzzz").is_err());
    }

    #[test]
    fn raw_binary_piece_placed_at_offset() {
        let dir = std::env::temp_dir().join(format!("lw-mi-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("raw.bin");
        std::fs::write(&path, [0xAA, 0xBB, 0xCC, 0xDD]).unwrap();

        let piece = load_flash_piece(&path, 0x1000).unwrap();
        assert_eq!(piece.segments.len(), 1);
        assert_eq!(piece.segments[0].start_addr, 0x1000);
        assert_eq!(piece.segments[0].data, vec![0xAA, 0xBB, 0xCC, 0xDD]);
    }

    #[test]
    fn ihex_piece_parsed_and_shifted() {
        let dir = std::env::temp_dir().join(format!("lw-mi-test-hex-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.hex");
        // One data record: 4 bytes DE AD BE EF at address 0x0010, then EOF.
        let hex = ":04001000DEADBEEF6E\n:00000001FF\n";
        std::fs::write(&path, hex).unwrap();

        let piece = load_flash_piece(&path, 0x2000).unwrap();
        assert_eq!(piece.segments.len(), 1);
        assert_eq!(piece.segments[0].start_addr, 0x2010);
        assert_eq!(piece.segments[0].data, vec![0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn ihex_extended_linear_address_applied() {
        let dir = std::env::temp_dir().join(format!("lw-mi-test-ela-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ela.hex");
        // Extended linear address record: upper 16 bits = 0x0001 -> base 0x00010000.
        // Then a data record at addr 0x0000 with 2 bytes.
        let hex = ":02000004000119\n:0200000001020A\n:00000001FF\n";
        std::fs::write(&path, hex).unwrap();

        let piece = load_flash_piece(&path, 0).unwrap();
        assert_eq!(piece.segments.len(), 1);
        assert_eq!(piece.segments[0].start_addr, 0x0001_0000);
        assert_eq!(piece.segments[0].data, vec![0x01, 0x02]);
    }

    #[test]
    fn overlap_detected_across_pieces() {
        let a = Segment {
            start_addr: 0x1000,
            data: vec![0u8; 0x100],
        };
        let b = Segment {
            start_addr: 0x1050,
            data: vec![0u8; 0x100],
        };
        let err = check_no_overlaps(&[a, b]).unwrap_err();
        assert!(err.to_string().contains("overlapping"));
    }

    #[test]
    fn adjacent_non_overlapping_segments_ok() {
        let a = Segment {
            start_addr: 0x1000,
            data: vec![0u8; 0x100],
        };
        let b = Segment {
            start_addr: 0x1100,
            data: vec![0u8; 0x100],
        };
        assert!(check_no_overlaps(&[a, b]).is_ok());
    }

    #[test]
    fn elf_and_raw_pieces_mix_without_overlap() {
        // ELF piece behaves like the raw one for the purposes of this test:
        // both eventually produce `Segment`s, so exercise the mixed Vec path
        // that the CLI composes directly.
        let elf_like = Segment {
            start_addr: 0x2_0000,
            data: vec![1, 2, 3, 4],
        };
        let raw = Segment {
            start_addr: 0x0,
            data: vec![5, 6, 7, 8],
        };
        assert!(check_no_overlaps(&[elf_like, raw]).is_ok());
    }
}
