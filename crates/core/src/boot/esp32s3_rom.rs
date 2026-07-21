// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 boot-ROM provisioning for the faithful `--rom-boot` path.
//!
//! The two flat images the model loads:
//!   * IROM (instruction bus) 0x4000_0000..0x4006_0000 (384 KiB)
//!   * DROM (data bus)        0x3FF0_0000..0x3FF2_0000 (128 KiB)
//!
//! Resolution order: explicit env pins, then the ROM ELF shipped with the
//! user's installed toolchain (PlatformIO/ESP-IDF), then the vendored images
//! under `crates/core/roms/esp32s3/` (embedded at build time, non-wasm only)
//! so `--rom-boot` works out of the box with no toolchain installed. The
//! vendored bins are extracted from Espressif's published
//! `esp32s3_rev0_rom.elf` (mask-ROM contents, Espressif copyright — see
//! `crates/core/roms/esp32s3/README.md` for provenance).
//!
//! This is a Rust port of `scripts/make_esp32s3_rom_bins.py`: PT_LOAD laid by
//! load-address (p_paddr) for IROM / vaddr for DROM, the boot ROM's `.data`
//! copy-source reconstruction, and a PROGBITS overlay for sections that live
//! in no PT_LOAD segment (e.g. `ets_rom_layout_p`).

use goblin::elf::program_header::PT_LOAD;
use goblin::elf::section_header::SHT_PROGBITS;
use goblin::elf::Elf;
use std::path::PathBuf;

pub const IROM_BASE: u32 = 0x4000_0000;
pub const IROM_SIZE: usize = 0x6_0000; // 384 KiB
pub const DROM_BASE: u32 = 0x3FF0_0000;
pub const DROM_SIZE: usize = 0x2_0000; // 128 KiB

const DRAM_LO: u32 = 0x3FC8_8000;
const DRAM_HI: u32 = 0x3FD0_0000;

/// Flat ROM images ready to load as `RamPeripheral`s at their window bases.
#[derive(Clone)]
pub struct RomImages {
    pub irom: Vec<u8>,
    pub drom: Vec<u8>,
}

impl std::fmt::Debug for RomImages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RomImages")
            .field("irom_len", &self.irom.len())
            .field("drom_len", &self.drom.len())
            .finish()
    }
}

/// The DRAM window the copy-table reconstruction targets — where the boot
/// ROM's `.data` lives at runtime. Chip-specific: the S3 range is below; the
/// C3 module passes its own range into these shared helpers.
#[derive(Clone, Copy)]
pub(crate) struct DramWindow {
    pub lo: u32,
    pub hi: u32,
}

const S3_DRAM: DramWindow = DramWindow {
    lo: DRAM_LO,
    hi: DRAM_HI,
};

/// Extract the IROM and DROM flat images from the genuine ROM ELF bytes.
pub fn extract_rom_images(elf_bytes: &[u8]) -> Result<RomImages, String> {
    let elf = Elf::parse(elf_bytes).map_err(|e| format!("parse ROM ELF: {e}"))?;
    let irom = build_window(&elf, elf_bytes, IROM_BASE, IROM_SIZE, true, S3_DRAM);
    let drom = build_window(&elf, elf_bytes, DROM_BASE, DROM_SIZE, false, S3_DRAM);
    Ok(RomImages { irom, drom })
}

pub(crate) fn build_window(
    elf: &Elf,
    bytes: &[u8],
    base: u32,
    size: usize,
    by_paddr: bool,
    dram: DramWindow,
) -> Vec<u8> {
    let mut img = vec![0u8; size];
    let win_end = base + size as u32;

    // 1. PT_LOAD pass — IROM keyed by load address (p_paddr), DROM by vaddr.
    for ph in &elf.program_headers {
        if ph.p_type != PT_LOAD || ph.p_filesz == 0 {
            continue;
        }
        let addr = if by_paddr {
            ph.p_paddr as u32
        } else {
            ph.p_vaddr as u32
        };
        if addr >= base && addr < win_end {
            let rel = (addr - base) as usize;
            let off = ph.p_offset as usize;
            let n = (ph.p_filesz as usize).min(size - rel);
            // Skip a segment whose file bytes are truncated/out-of-file (defensive; a genuine ROM ELF never hits this).
            if off + n <= bytes.len() {
                img[rel..rel + n].copy_from_slice(&bytes[off..off + n]);
            }
        }
    }

    // 2. Reconstruct the boot ROM's `.data` copy sources (IROM window only).
    if by_paddr {
        populate_data_copy_sources(elf, bytes, &mut img, base, dram);
    }

    // 3. Overlay PROGBITS sections that live in this window but in no PT_LOAD
    //    segment (e.g. the DROM `.rodata.interface` holding `ets_rom_layout_p`).
    //    Only fill bytes the PT_LOAD pass left as zero.
    for (sh_addr, data) in progbits_sections(elf, bytes) {
        if sh_addr >= base && sh_addr < win_end {
            let rel = (sh_addr - base) as usize;
            let n = data.len().min(size - rel);
            for i in 0..n {
                if img[rel + i] == 0 {
                    img[rel + i] = data[i];
                }
            }
        }
    }

    img
}

/// (sh_addr, bytes) for every SHT_PROGBITS section with an address + content,
/// sorted by address.
fn progbits_sections<'a>(elf: &Elf, bytes: &'a [u8]) -> Vec<(u32, &'a [u8])> {
    let mut v: Vec<(u32, &[u8])> = Vec::new();
    for sh in &elf.section_headers {
        if sh.sh_type == SHT_PROGBITS && sh.sh_size != 0 && sh.sh_addr != 0 {
            let off = sh.sh_offset as usize;
            let sz = sh.sh_size as usize;
            if off + sz <= bytes.len() {
                v.push((sh.sh_addr as u32, &bytes[off..off + sz]));
            }
        }
    }
    v.sort_by_key(|(a, _)| *a);
    v
}

/// Walk the in-image 16-byte copy-table quads (dst_start, dst_end, src, 0) and
/// fill each `src` LMA in the IROM image with the genuine bytes the matching
/// DRAM `dst` section holds, so the ROM's own startup copy lands real values.
fn populate_data_copy_sources(
    elf: &Elf,
    bytes: &[u8],
    irom: &mut [u8],
    irom_base: u32,
    dram: DramWindow,
) {
    let sections = progbits_sections(elf, bytes);
    let vma_read = |addr: u32, n: usize| -> Vec<u8> {
        let mut out = vec![0u8; n];
        for (sa, data) in &sections {
            let sa = *sa;
            let end = sa as u64 + data.len() as u64;
            if (sa as u64) <= addr as u64 + n as u64 && (addr as u64) < end {
                // lo/hi are proven in-range once the overlap check above passes.
                let lo = addr.max(sa);
                let hi = (addr + n as u32).min(end as u32);
                out[(lo - addr) as usize..(hi - addr) as usize]
                    .copy_from_slice(&data[(lo - sa) as usize..(hi - sa) as usize]);
            }
        }
        out
    };

    let irom_hi = irom_base + irom.len() as u32;
    let mut off = 0usize;
    while off + 16 <= irom.len() {
        let dst_s = u32::from_le_bytes(irom[off..off + 4].try_into().unwrap());
        let dst_e = u32::from_le_bytes(irom[off + 4..off + 8].try_into().unwrap());
        let src = u32::from_le_bytes(irom[off + 8..off + 12].try_into().unwrap());
        let term = u32::from_le_bytes(irom[off + 12..off + 16].try_into().unwrap());
        // NB: `dst_e <= dram.hi`, inclusive — a copy record's end is exclusive
        // and the last record may end exactly at the DRAM top (the C3's
        // `ets_ops_table_ptr` sits at 0x3FCDFFFC..0x3FCE0000; rejecting it
        // leaves the ROM's ops table null and boot dies in
        // ets_run_flash_bootloader with a jalr to 0).
        let ok = (dram.lo..dram.hi).contains(&dst_s)
            && dst_s <= dst_e
            && dst_e <= dram.hi
            && irom_base <= src
            && src < irom_hi
            && term == 0
            && dst_e - dst_s < 0x1_0000;
        if ok {
            let n = (dst_e - dst_s) as usize;
            if n != 0 {
                let vals = vma_read(dst_s, n);
                if vals.iter().any(|&b| b != 0) {
                    let rel = (src - irom_base) as usize;
                    let n2 = n.min(irom.len().saturating_sub(rel));
                    irom[rel..rel + n2].copy_from_slice(&vals[..n2]);
                }
            }
            off += 16;
        } else {
            off += 4;
        }
    }
}

/// Walk the boot ROM's `.data` copy table and return its records as
/// `(dst_addr, byte_len, src_offset_in_irom)`. Uses the identical 16-byte-quad
/// (dst_start, dst_end, src, 0) scan and validity heuristic as
/// `populate_data_copy_sources`, so the two stay in lockstep.
pub(crate) fn copy_table_records(
    irom: &[u8],
    irom_base: u32,
    dram: DramWindow,
) -> Vec<(u32, usize, usize)> {
    let irom_hi = irom_base + irom.len() as u32;
    let mut recs = Vec::new();
    let mut off = 0usize;
    while off + 16 <= irom.len() {
        let dst_s = u32::from_le_bytes(irom[off..off + 4].try_into().unwrap());
        let dst_e = u32::from_le_bytes(irom[off + 4..off + 8].try_into().unwrap());
        let src = u32::from_le_bytes(irom[off + 8..off + 12].try_into().unwrap());
        let term = u32::from_le_bytes(irom[off + 12..off + 16].try_into().unwrap());
        let ok = (dram.lo..dram.hi).contains(&dst_s)
            && dst_s <= dst_e
            && dst_e <= dram.hi
            && irom_base <= src
            && src < irom_hi
            && term == 0
            && dst_e - dst_s < 0x1_0000;
        if ok {
            let n = (dst_e - dst_s) as usize;
            if n != 0 {
                recs.push((dst_s, n, (src - irom_base) as usize));
            }
            off += 16;
        } else {
            off += 4;
        }
    }
    recs
}

/// Replicate the ESP32-S3 boot ROM's reset-time `.data` initialization: for
/// each record in the ROM's copy table, return the genuine source bytes (put in
/// the IROM image by `populate_data_copy_sources` / extraction) paired with
/// their DRAM destination, for the caller to write onto the bus.
///
/// Fast-boot jumps straight into the app and skips the ROM's own startup, so
/// without this the ROM's DRAM data structures stay zero. The concrete failure
/// that motivated it: `rom_cache_internal_table_ptr` @0x3FCEFFC4 stays null, so
/// the real ROM cache helpers esp-hal's `pre_init` calls
/// (`rom_config_instruction_cache_mode` etc.) dispatch through
/// `table[0] -> [+16]` == 0 → `callx8 a8` with a8 == 0 → jump to 0x0. Running
/// this copy lands the real table pointer (0x3FF1E2B4, in mapped DROM) so those
/// helpers execute faithfully — zero thunks. Idempotent with a real reset boot,
/// which performs the same copy itself.
pub fn s3_rom_data_init_writes(irom: &[u8]) -> Vec<(u32, Vec<u8>)> {
    rom_data_init_writes(irom, IROM_BASE, S3_DRAM)
}

/// Shared back-end for [`s3_rom_data_init_writes`] and the ESP32-C3 analogue
/// [`super::esp32c3_rom::c3_rom_data_init_writes`]: walk the ROM's `.data` copy
/// table and pair each record's genuine source bytes with its DRAM destination.
/// The two chips use the same reset-time copy-table format and the same IROM
/// base; only the DRAM window differs.
pub(crate) fn rom_data_init_writes(
    irom: &[u8],
    irom_base: u32,
    dram: DramWindow,
) -> Vec<(u32, Vec<u8>)> {
    copy_table_records(irom, irom_base, dram)
        .into_iter()
        .filter_map(|(dst, n, src_off)| {
            let end = src_off.checked_add(n)?;
            (end <= irom.len()).then(|| (dst, irom[src_off..end].to_vec()))
        })
        .collect()
}

/// Resolve the ROM images for the faithful path, or `None` to fall back to
/// the thunk harness.
///
/// Order:
///   0. If `LABWIRED_ESP32S3_FASTBOOT` is set (any value), immediately return
///      `None` to force the fast-boot/thunk path regardless of toolchain
///      availability. Use this for the playground fast-boot path and for unit
///      tests that assert fast-boot-specific wiring.
///   1. Explicit pre-extracted `LABWIRED_ESP32S3_ROM`/`_DROM` bins (back-compat).
///   2. Discover the toolchain ROM ELF, extract (cached by ELF content hash), load.
///   3. Vendored images embedded at build time (non-wasm only) — the
///      out-of-the-box path when no toolchain is installed.
pub fn provision_rom_images() -> Option<RomImages> {
    // 0. Explicit opt-out: force fast-boot/harness even when a real ROM is
    //    available (used by the fast-boot playground path and by unit tests that
    //    assert fast-boot-specific wiring). Set LABWIRED_ESP32S3_FASTBOOT=1.
    if std::env::var_os("LABWIRED_ESP32S3_FASTBOOT").is_some() {
        return None;
    }

    // 1. Back-compat: explicit pre-extracted flat bins still win.
    if let (Ok(rp), Ok(dp)) = (
        std::env::var("LABWIRED_ESP32S3_ROM"),
        std::env::var("LABWIRED_ESP32S3_DROM"),
    ) {
        if let (Ok(irom), Ok(drom)) = (std::fs::read(&rp), std::fs::read(&dp)) {
            return Some(RomImages { irom, drom });
        }
    }

    // 2. Discover + extract (cached).
    if let Some(elf_path) = discover_rom_elf() {
        if let Ok(elf_bytes) = std::fs::read(&elf_path) {
            let key = fnv1a_64(&elf_bytes);
            let dir = cache_dir();
            let irom_path = dir.join(format!("esp32s3_irom_{key:016x}.bin"));
            let drom_path = dir.join(format!("esp32s3_drom_{key:016x}.bin"));

            if let (Ok(irom), Ok(drom)) = (std::fs::read(&irom_path), std::fs::read(&drom_path)) {
                if irom.len() == IROM_SIZE && drom.len() == DROM_SIZE {
                    return Some(RomImages { irom, drom });
                }
            }

            if let Ok(images) = extract_rom_images(&elf_bytes) {
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::fs::write(&irom_path, &images.irom);
                let _ = std::fs::write(&drom_path, &images.drom);
                return Some(images);
            }
        }
    }

    // 3. Vendored images — out-of-the-box fallback (no toolchain needed).
    vendored_rom_images()
}

/// Vendored flat ROM images, embedded at build time. Extracted from
/// Espressif's published `esp32s3_rev0_rom.elf` by
/// `scripts/make_esp32s3_rom_bins.py` — see `crates/core/roms/esp32s3/`.
///
/// Native only — the 512 KiB is NOT baked into wasm bundles. The playground
/// fetches the ROM as an on-demand asset and injects it via
/// `Esp32s3Opts.rom_images` (see `configure_esp32s3_memmap`), so the shared
/// wasm engine stays lean for every non-S3 board.
#[cfg(not(target_arch = "wasm32"))]
fn vendored_rom_images() -> Option<RomImages> {
    static VENDORED_IROM: &[u8] = include_bytes!("../../roms/esp32s3/esp32s3_rom.bin");
    static VENDORED_DROM: &[u8] = include_bytes!("../../roms/esp32s3/esp32s3_drom.bin");
    debug_assert_eq!(VENDORED_IROM.len(), IROM_SIZE);
    debug_assert_eq!(VENDORED_DROM.len(), DROM_SIZE);
    Some(RomImages {
        irom: VENDORED_IROM.to_vec(),
        drom: VENDORED_DROM.to_vec(),
    })
}

/// On wasm the ROM is never baked in; it arrives via `Esp32s3Opts.rom_images`.
#[cfg(target_arch = "wasm32")]
fn vendored_rom_images() -> Option<RomImages> {
    None
}

/// Cache directory for extracted ROM images (`$XDG_CACHE_HOME` or `~/.cache`).
fn cache_dir() -> PathBuf {
    if let Ok(x) = std::env::var("XDG_CACHE_HOME") {
        return PathBuf::from(x).join("labwired/esp32s3-rom");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".cache/labwired/esp32s3-rom");
    }
    std::env::temp_dir().join("labwired-esp32s3-rom")
}

/// FNV-1a 64-bit hash (no extra deps; stable cache key across runs).
fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Locate the genuine ESP32-S3 ROM ELF in the user's installed toolchain.
///
/// Preference order:
///   1. `LABWIRED_ESP32S3_ROM_ELF` env var (explicit path).
///   2. PlatformIO: `~/.platformio/tools/tool-esp-rom-elfs/esp32s3_rev0_rom.elf`.
///   3. ESP-IDF: `~/.espressif/tools/esp-rom-elfs/<ver>/esp32s3_rev0_rom.elf`.
pub(crate) fn discover_rom_elf() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("LABWIRED_ESP32S3_ROM_ELF") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let home = std::env::var("HOME").ok()?;

    let pio = PathBuf::from(format!(
        "{home}/.platformio/tools/tool-esp-rom-elfs/esp32s3_rev0_rom.elf"
    ));
    if pio.is_file() {
        return Some(pio);
    }

    // ESP-IDF nests the elfs under a version directory; scan one level.
    let idf_root = PathBuf::from(format!("{home}/.espressif/tools/esp-rom-elfs"));
    if let Ok(entries) = std::fs::read_dir(&idf_root) {
        for entry in entries.flatten() {
            let candidate = entry.path().join("esp32s3_rev0_rom.elf");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    // Serialises tests that mutate process-wide LABWIRED_ESP32S3_* env vars,
    // since cargo runs tests in parallel threads.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Minimal little-endian ELF32 with one PT_LOAD program header, used to
    /// verify the window-builder places file bytes at the right window offset.
    fn synthetic_elf_one_ptload(vaddr: u32, paddr: u32, payload: &[u8]) -> Vec<u8> {
        // Layout: [ehdr 52][phdr 32][payload]
        let e_phoff = 52u32;
        let e_phentsize = 32u16;
        let p_offset = (52 + 32) as u32;
        let mut elf = vec![0u8; p_offset as usize + payload.len()];
        elf[0..4].copy_from_slice(b"\x7fELF");
        elf[4] = 1; // ELFCLASS32
        elf[5] = 1; // little-endian
        elf[6] = 1; // version
                    // e_type=ET_EXEC(2), e_machine=94 (Xtensa), e_version=1
        elf[16..18].copy_from_slice(&2u16.to_le_bytes());
        elf[18..20].copy_from_slice(&94u16.to_le_bytes());
        elf[20..24].copy_from_slice(&1u32.to_le_bytes());
        elf[28..32].copy_from_slice(&e_phoff.to_le_bytes()); // e_phoff
        elf[42..44].copy_from_slice(&e_phentsize.to_le_bytes()); // e_phentsize
        elf[44..46].copy_from_slice(&1u16.to_le_bytes()); // e_phnum = 1
                                                          // program header (ELF32): type, offset, vaddr, paddr, filesz, memsz, flags, align
        let ph = e_phoff as usize;
        elf[ph..ph + 4].copy_from_slice(&1u32.to_le_bytes()); // PT_LOAD
        elf[ph + 4..ph + 8].copy_from_slice(&p_offset.to_le_bytes());
        elf[ph + 8..ph + 12].copy_from_slice(&vaddr.to_le_bytes());
        elf[ph + 12..ph + 16].copy_from_slice(&paddr.to_le_bytes());
        elf[ph + 16..ph + 20].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        elf[ph + 20..ph + 24].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        elf[ph + 24..ph + 28].copy_from_slice(&4u32.to_le_bytes());
        elf[ph + 28..ph + 32].copy_from_slice(&4u32.to_le_bytes());
        elf[p_offset as usize..].copy_from_slice(payload);
        elf
    }

    /// Build a minimal ELF32 with one PT_LOAD **and** one SHT_PROGBITS section.
    ///
    /// Layout (all offsets are fixed / pre-calculated):
    ///   offset   0: ELF header (52 bytes)
    ///   offset  52: program header (32 bytes)
    ///   offset  84: PT_LOAD payload (`ph_payload`, length ph_payload.len())
    ///   offset  84+P: PROGBITS section data (`sh_payload`, length sh_payload.len())
    ///   offset  84+P+S: section name string table ("\0.text\0", 8 bytes)
    ///   offset  84+P+S+8: section headers — [null(40), PROGBITS(40), STRTAB(40)]
    ///                      i.e. 3 × 40 = 120 bytes
    ///
    /// Returns the raw ELF bytes.
    fn synthetic_elf_ptload_and_section(
        ph_vaddr: u32,
        ph_paddr: u32,
        ph_payload: &[u8],
        sh_addr: u32,
        sh_payload: &[u8],
    ) -> Vec<u8> {
        let p = ph_payload.len();
        let s = sh_payload.len();

        // Fixed layout positions
        let e_phoff: u32 = 52;
        let ph_data_off: u32 = 52 + 32; // = 84
        let sh_data_off: u32 = ph_data_off + p as u32;
        let strtab_off: u32 = sh_data_off + s as u32;
        let strtab: &[u8] = b"\0.text\0"; // 7 bytes; ".text" at offset 1
        let strtab_len: u32 = strtab.len() as u32;
        let e_shoff: u32 = strtab_off + strtab_len;
        let total = (e_shoff + 3 * 40) as usize;

        let mut elf = vec![0u8; total];

        // ---- ELF header ----
        elf[0..4].copy_from_slice(b"\x7fELF");
        elf[4] = 1; // ELFCLASS32
        elf[5] = 1; // little-endian
        elf[6] = 1; // EI_VERSION
        elf[16..18].copy_from_slice(&2u16.to_le_bytes()); // ET_EXEC
        elf[18..20].copy_from_slice(&94u16.to_le_bytes()); // EM_XTENSA
        elf[20..24].copy_from_slice(&1u32.to_le_bytes()); // e_version
        elf[28..32].copy_from_slice(&e_phoff.to_le_bytes()); // e_phoff
        elf[32..36].copy_from_slice(&e_shoff.to_le_bytes()); // e_shoff
        elf[40..42].copy_from_slice(&52u16.to_le_bytes()); // e_ehsize
        elf[42..44].copy_from_slice(&32u16.to_le_bytes()); // e_phentsize
        elf[44..46].copy_from_slice(&1u16.to_le_bytes()); // e_phnum
        elf[46..48].copy_from_slice(&40u16.to_le_bytes()); // e_shentsize
        elf[48..50].copy_from_slice(&3u16.to_le_bytes()); // e_shnum (null + progbits + strtab)
        elf[50..52].copy_from_slice(&2u16.to_le_bytes()); // e_shstrndx = 2

        // ---- Program header (PT_LOAD) ----
        let ph = e_phoff as usize;
        elf[ph..ph + 4].copy_from_slice(&1u32.to_le_bytes()); // PT_LOAD
        elf[ph + 4..ph + 8].copy_from_slice(&ph_data_off.to_le_bytes()); // p_offset
        elf[ph + 8..ph + 12].copy_from_slice(&ph_vaddr.to_le_bytes()); // p_vaddr
        elf[ph + 12..ph + 16].copy_from_slice(&ph_paddr.to_le_bytes()); // p_paddr
        elf[ph + 16..ph + 20].copy_from_slice(&(p as u32).to_le_bytes()); // p_filesz
        elf[ph + 20..ph + 24].copy_from_slice(&(p as u32).to_le_bytes()); // p_memsz
        elf[ph + 24..ph + 28].copy_from_slice(&5u32.to_le_bytes()); // flags R|X
        elf[ph + 28..ph + 32].copy_from_slice(&4u32.to_le_bytes()); // align

        // ---- PT_LOAD payload ----
        elf[ph_data_off as usize..ph_data_off as usize + p].copy_from_slice(ph_payload);

        // ---- PROGBITS section payload ----
        elf[sh_data_off as usize..sh_data_off as usize + s].copy_from_slice(sh_payload);

        // ---- String table ----
        elf[strtab_off as usize..strtab_off as usize + strtab_len as usize].copy_from_slice(strtab);

        // ---- Section headers ----
        // [0] SHT_NULL — all zeros (already zero)

        // [1] SHT_PROGBITS
        let sh1 = e_shoff as usize + 40; // index 1
        elf[sh1..sh1 + 4].copy_from_slice(&1u32.to_le_bytes()); // sh_name = offset 1 → ".text"
        elf[sh1 + 4..sh1 + 8].copy_from_slice(&1u32.to_le_bytes()); // sh_type = SHT_PROGBITS
        elf[sh1 + 8..sh1 + 12].copy_from_slice(&2u32.to_le_bytes()); // sh_flags = SHF_ALLOC
        elf[sh1 + 12..sh1 + 16].copy_from_slice(&sh_addr.to_le_bytes()); // sh_addr
        elf[sh1 + 16..sh1 + 20].copy_from_slice(&sh_data_off.to_le_bytes()); // sh_offset
        elf[sh1 + 20..sh1 + 24].copy_from_slice(&(s as u32).to_le_bytes()); // sh_size

        // [2] SHT_STRTAB
        let sh2 = e_shoff as usize + 80; // index 2
        elf[sh2..sh2 + 4].copy_from_slice(&0u32.to_le_bytes()); // sh_name = 0 → ""
        elf[sh2 + 4..sh2 + 8].copy_from_slice(&3u32.to_le_bytes()); // sh_type = SHT_STRTAB
        elf[sh2 + 16..sh2 + 20].copy_from_slice(&strtab_off.to_le_bytes()); // sh_offset
        elf[sh2 + 20..sh2 + 24].copy_from_slice(&strtab_len.to_le_bytes()); // sh_size

        elf
    }

    #[test]
    fn overlay_only_fills_zero_bytes() {
        // DROM window: base = DROM_BASE, keyed by vaddr.
        // PT_LOAD covers [DROM_BASE+0x100 .. DROM_BASE+0x108] with bytes 0xAA..
        // PROGBITS section covers [DROM_BASE+0x100 .. DROM_BASE+0x10C]:
        //   first 8 bytes overlap PT_LOAD (should NOT be clobbered),
        //   last 4 bytes are outside PT_LOAD (zero → should be filled from section).
        let ph_payload = [0xAAu8; 8];
        let sh_payload = [0xBBu8; 12]; // overlaps PT_LOAD for first 8, extends 4 more
        let sh_addr = DROM_BASE + 0x100;
        let ph_vaddr = DROM_BASE + 0x100;
        let ph_paddr = 0xDEAD_0000u32; // irrelevant for DROM (vaddr-keyed)

        let elf_bytes =
            synthetic_elf_ptload_and_section(ph_vaddr, ph_paddr, &ph_payload, sh_addr, &sh_payload);
        let images = extract_rom_images(&elf_bytes).expect("extract");

        // PT_LOAD bytes must NOT be overwritten by the overlay
        assert_eq!(
            &images.drom[0x100..0x108],
            &[0xAAu8; 8],
            "PT_LOAD bytes must not be clobbered by overlay"
        );
        // The 4 bytes beyond the PT_LOAD (zero region) should be filled from the section
        assert_eq!(
            &images.drom[0x108..0x10C],
            &[0xBBu8; 4],
            "zero bytes after PT_LOAD region should be filled from PROGBITS overlay"
        );
    }

    #[test]
    fn copy_table_reconstructs_data_source() {
        // Craft an IROM-keyed ELF (by_paddr = true) whose PT_LOAD payload contains
        // a valid copy-table quad followed by arbitrary data at the `src` position.
        //
        // Copy-table quad:
        //   dst_s = DRAM_LO           (= 0x3FC8_8000)
        //   dst_e = DRAM_LO + 8       (copy 8 bytes)
        //   src   = IROM_BASE + 0x20  (source in IROM window)
        //   term  = 0
        //
        // A PROGBITS section lives at sh_addr = dst_s = DRAM_LO and holds
        // SECTION_BYTES (non-zero).  The copy-table logic should read those bytes
        // via vma_read(dst_s, 8) and write them into irom[0x20..0x28].

        const SECTION_BYTES: [u8; 8] = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];

        // Build the PT_LOAD payload (must be large enough to hold the quad at
        // offset 0x00 and the destination slot at offset 0x20).
        // Layout within payload: [16-byte quad][padding][8-byte src slot]
        // quad starts at payload[0], src slot at payload[0x20].
        // Total payload: 0x28 bytes.
        let mut ph_payload = vec![0u8; 0x28];

        let dst_s: u32 = DRAM_LO;
        let dst_e: u32 = DRAM_LO + 8;
        let src: u32 = IROM_BASE + 0x20;

        ph_payload[0..4].copy_from_slice(&dst_s.to_le_bytes());
        ph_payload[4..8].copy_from_slice(&dst_e.to_le_bytes());
        ph_payload[8..12].copy_from_slice(&src.to_le_bytes());
        ph_payload[12..16].copy_from_slice(&0u32.to_le_bytes()); // term = 0
                                                                 // src slot at payload[0x20..0x28] — left as zero initially

        // ph_paddr = IROM_BASE (so the PT_LOAD lands at window offset 0)
        // ph_vaddr = anything outside DROM (so DROM doesn't see it)
        let ph_vaddr: u32 = 0xDEAD_0000;
        let ph_paddr: u32 = IROM_BASE;

        // PROGBITS section at dst_s = DRAM_LO holding SECTION_BYTES
        let elf_bytes = synthetic_elf_ptload_and_section(
            ph_vaddr,
            ph_paddr,
            &ph_payload,
            dst_s,
            &SECTION_BYTES,
        );

        let images = extract_rom_images(&elf_bytes).expect("extract");

        // After copy-table reconstruction, irom[0x20..0x28] should hold SECTION_BYTES
        assert_eq!(
            &images.irom[0x20..0x28],
            &SECTION_BYTES,
            "copy-table source slot in IROM must be populated from PROGBITS section bytes"
        );
    }

    #[test]
    fn irom_window_keyed_by_paddr() {
        // A segment whose paddr is in the IROM window but vaddr is elsewhere
        // (mirrors the ROM's .data stored at an IROM LMA) must land in IROM.
        let payload = [0xAA, 0xBB, 0xCC, 0xDD];
        let elf = synthetic_elf_one_ptload(0x3FCD_7E00, IROM_BASE + 0x100, &payload);
        let images = extract_rom_images(&elf).expect("extract");
        assert_eq!(images.irom.len(), IROM_SIZE);
        assert_eq!(&images.irom[0x100..0x104], &payload);
    }

    #[test]
    fn drom_window_keyed_by_vaddr() {
        let payload = [0x11, 0x22, 0x33, 0x44];
        let elf = synthetic_elf_one_ptload(DROM_BASE + 0x200, 0xDEAD_0000, &payload);
        let images = extract_rom_images(&elf).expect("extract");
        assert_eq!(images.drom.len(), DROM_SIZE);
        assert_eq!(&images.drom[0x200..0x204], &payload);
    }

    #[test]
    fn explicit_env_override_wins_for_discovery() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // A path set via LABWIRED_ESP32S3_ROM_ELF that exists is returned as-is.
        let tmp = std::env::temp_dir().join("labwired_test_rom_elf.bin");
        std::fs::write(&tmp, b"\x7fELF").unwrap();
        std::env::set_var("LABWIRED_ESP32S3_ROM_ELF", &tmp);
        let found = discover_rom_elf().expect("env override should resolve");
        assert_eq!(found, tmp);
        std::env::remove_var("LABWIRED_ESP32S3_ROM_ELF");
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn provision_extracts_and_caches_from_elf_path() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let payload = [0x5A, 0x5B, 0x5C, 0x5D];
        let elf = synthetic_elf_one_ptload(IROM_BASE + 0x40, IROM_BASE + 0x40, &payload);
        let tmp = std::env::temp_dir().join("labwired_test_provision_rom.elf");
        std::fs::write(&tmp, &elf).unwrap();
        std::env::set_var("LABWIRED_ESP32S3_ROM_ELF", &tmp);
        // Ensure the env pre-extracted-bins path is not taken.
        std::env::remove_var("LABWIRED_ESP32S3_ROM");
        std::env::remove_var("LABWIRED_ESP32S3_DROM");

        let images = provision_rom_images().expect("provision");
        assert_eq!(images.irom.len(), IROM_SIZE);
        assert_eq!(&images.irom[0x40..0x44], &payload);

        std::env::remove_var("LABWIRED_ESP32S3_ROM_ELF");
        let _ = std::fs::remove_file(&tmp);
    }

    /// Out-of-the-box guarantee: with no env pins and no toolchain ELF, the
    /// vendored images must resolve with the architected window sizes and the
    /// BROM reset-vector code present (non-zero bytes at 0x400).
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn vendored_images_resolve_with_architected_sizes() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let images = vendored_rom_images().expect("vendored ROM images embedded");
        assert_eq!(images.irom.len(), IROM_SIZE);
        assert_eq!(images.drom.len(), DROM_SIZE);
        // Reset vector 0x40000400 (IROM offset 0x400) holds real code.
        assert!(images.irom[0x400..0x410].iter().any(|&b| b != 0));
    }
}
