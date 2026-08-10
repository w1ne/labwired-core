// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! THE CONTRACT: a peripheral's **base address** belongs to the chip YAML.
//! A peripheral model may not keep a second copy of it in Rust.
//!
//! # The bug class
//!
//! A base address hardcoded in a model is a second home for a fact the chip
//! descriptor already owns, with nothing forcing the two to agree. When they
//! disagree the failure is **silent**, because a wrong address is still a
//! *valid* address: it lands in some other peripheral's MMIO window and is
//! absorbed without a fault, a log line, or a test failure.
//!
//! Two confirmed instances, both of which cost real time:
//!
//! * **nRF52 GPIOTE** hardcoded `GPIO1_BASE = 0x5000_0300`, Nordic's
//!   real-silicon P1 base. Every nRF52840 chip YAML deliberately remaps `gpio1`
//!   to `0x5000_1000`, because the silicon base sits *inside* GPIO0's 4 KB
//!   window and a flat bus cannot host both. So a port-1 GPIOTE task wrote
//!   `0x5000_0810` — inside **gpio0's** window, where it vanished. The model's
//!   own unit test asserted the wrong base, so it passed while the behaviour
//!   was broken.
//! * **classic ESP32 `apb_ctrl` vs SYSCON** resolved to the same base. Bus
//!   routing ties are won by the LAST peripheral registered, so `apb_ctrl`
//!   shadowed SYSCON, `SYSCLK_CONF` read `0xFFFF_FFFF`, `getApbFrequency()`
//!   returned 78125 Hz and the Arduino baud divisor blew up. "Serial does not
//!   work on classic ESP32" was accepted as a property of the product for
//!   about a year. It was never a UART bug — it was an address-map
//!   disagreement that failed silently.
//!
//! # Where the line is drawn — deliberately
//!
//! * A peripheral's **BASE address** comes from the memory map. That is the
//!   chip YAML's job. → **Rule 1** below.
//! * A **register OFFSET within** a peripheral (`+0x510` for GPIO.IN) is a
//!   silicon fact of that peripheral and legitimately lives with the model.
//!   Offsets are NOT checked and must not be mass-migrated to YAML: that would
//!   scatter one peripheral's register map across two files and be worse than
//!   the disease.
//! * Memory **regions** (flash/RAM/ROM/XIP windows, DMA scratch buffers) are
//!   not peripherals and are out of scope here.
//!
//! # The two rules
//!
//! * **Rule 1 — no hardcoded base in a model.** No production source under
//!   `crates/core/src/peripherals/` may bind a `const`/`static` whose *name*
//!   says it is a base or an address (`*BASE*`, `*ADDR*`) to an absolute MMIO
//!   literal (`>= 0x1000_0000`). Models that need an address must read it from
//!   the chip descriptor — their own via `p_cfg.base_address`, a sibling's via
//!   [`crate::peripherals::chip_map::ChipMap`].
//!
//!   Note this rule keys on the **shape of the binding**, not on whether the
//!   value matches a YAML entry — on purpose. The GPIOTE constant
//!   `0x5000_0300` matched *no* declared base anywhere in the repo; that
//!   disagreement was the entire bug. A rule of the form "flag literals equal
//!   to a declared base" would have missed it completely.
//!
//! * **Rule 2 — a chip YAML may not disagree with itself.** Within one chip
//!   descriptor, no two peripherals may share a `base_address`, and no two
//!   `[base, base + size)` windows may overlap. An overlap is resolved
//!   silently by registration order, which is how `apb_ctrl` ate SYSCON.
//!
//! # What this gate does NOT catch
//!
//! Stated plainly so nobody reads a green run as more than it is:
//!
//! * **Inline literals.** `bus.write_u32(0x6000_0000 + 0x1C, v)` with no named
//!   binding is invisible to Rule 1. Covering those means flagging ~265 sites,
//!   the large majority of which are doc comments, test fixtures and
//!   `base + offset` composites — an allowlist of that size stops being read.
//!   Rule 1 targets the shape both confirmed defects actually had.
//! * **Wrong-but-consistent maps.** If the YAML itself declares the wrong
//!   address, everything here is green and the model is still wrong. This gate
//!   enforces *one home*, not correctness of the value in that home.
//! * **Cross-chip register-map assumptions.** Rule 2 is per-chip-file by
//!   design. The same offset does NOT mean the same register across ESP32-C3
//!   and S3, and this repo has already been burned by an inherited shared map.
//!   Nothing here should be read as licence to share one.
//!
//! # Allowlists shrink, never grow
//!
//! Every entry is re-checked: an entry that no longer violates fails the gate,
//! so a converted model cannot leave a stale exemption behind for the next
//! offender to hide under.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

// ─────────────────────────────────────────────────────────────────────────────
// Frozen allowlists. Adding a line here is a deliberate act; read the rule
// docs above before you do it.
// ─────────────────────────────────────────────────────────────────────────────

/// Rule 1 exemptions: production `*BASE*` / `*ADDR*` constants still holding an
/// absolute MMIO literal.
///
/// Every entry below is a live instance of the same class as the nRF52 GPIOTE
/// defect this contract was written for. They are recorded rather than
/// converted because each needs its own plumbing decision and its own
/// behavioural proof, and because several sit on the ESP32-C3/S3 throughput
/// paths where a byte-identical result has to be demonstrated, not assumed.
///
/// Severity note: the dangerous shape is a model addressing a **different**
/// peripheral (what GPIOTE did). A model naming its **own** base is a
/// duplicated fact but a far less explosive one — it is normally also passed
/// `p_cfg.base_address`, so the two can be reconciled without new plumbing.
/// Entries are annotated with which kind they are.
///
/// Format: `(path relative to crates/core/src, const name, why it is still here)`.
const HARDCODED_BASE_ALLOWLIST: &[(&str, &str, &str)] = &[
    // ── cross-peripheral emitters: same shape as the GPIOTE defect ──────────
    (
        "peripherals/esp32s3/gdma.rs",
        "UART0_BASE",
        "CROSS-PERIPHERAL, highest-value next conversion. GDMA/UHCI0 reads and \
         writes UART0's FIFO (+0x00) and STATUS (+0x1C) at a hardcoded base — \
         exactly the shape that broke GPIOTE. Both esp32s3 YAMLs happen to \
         declare uart0 at 0x6000_0000 so it is currently correct, and nothing \
         enforces that. Converting needs ChipMap threaded through \
         esp32s3::factory (two callers, one of which builds no chip YAML) and \
         touches ~70 Esp32s3Gdma::new call sites.",
    ),
    (
        "peripherals/esp32s3/crosscore_ipi.rs",
        "BASE",
        "CROSS-PERIPHERAL. 0x600C_0030 is an offset INTO the `system` window, \
         not a peripheral base of its own — no chip YAML declares it, so there \
         is no id to resolve. Needs the system base from ChipMap plus a named \
         offset; left alone rather than guessed at.",
    ),
    // ── register field encodings that merely LOOK like bases ────────────────
    (
        "peripherals/esp32c3/pms.rs",
        "IRAM0_STATUS_ADDR_BASE",
        "NOT A PERIPHERAL BASE. `IRAM0_VIOLATE_STATUS_ADDR_OFFSET` from IDF's \
         soc/esp32c3/memprot_defs.h: the PMS stores a violating address in \
         CORE_0_IRAM0_PMS_MONITOR_2[28:5] relative to 0x4000_0000, and IDF's \
         own `memprot_ll_iram0_get_monitor_status_fault_addr` adds it back. It \
         is part of a register's field encoding, like a shift or a mask — no \
         chip YAML declares it and there is nothing for ChipMap to resolve.",
    ),
    (
        "peripherals/esp32c3/pms.rs",
        "DRAM0_STATUS_ADDR_BASE",
        "NOT A PERIPHERAL BASE. `DRAM0_VIOLATE_STATUS_ADDR_OFFSET`, the DRAM0 \
         twin of the above (CORE_0_DRAM0_PMS_MONITOR_2[27:4] is stored relative \
         to 0x3C00_0000). Same reasoning.",
    ),
    // ── models naming their own base ────────────────────────────────────────
    (
        "peripherals/esp32c3/apb_saradc.rs",
        "APB_SARADC_BASE",
        "OWN BASE (esp32c3.yaml: apb_saradc). Exported for the C3 system \
         builder's registration call; reconciling it means having that builder \
         read the descriptor. C3 throughput is a byte-identical gate, so this \
         needs its own proof run.",
    ),
    (
        "peripherals/esp32c3/bt.rs",
        "BT_BASE",
        "OWN BASE (esp32c3.yaml: bt). The C3 BT block is on the validated BLE \
         path; touching its registration without a live BLE re-run is not worth \
         the risk for a duplicated constant.",
    ),
    (
        "peripherals/esp32c3/i2c.rs",
        "I2C0_BASE",
        "OWN BASE (esp32c3.yaml: i2c0). On the C3 shipped-lab throughput path.",
    ),
    (
        "peripherals/esp32c3/ledc.rs",
        "LEDC_BASE",
        "OWN BASE (esp32c3.yaml: ledc).",
    ),
    (
        "peripherals/esp32c3/spi.rs",
        "SPI2_BASE",
        "OWN BASE (esp32c3.yaml: spi2).",
    ),
    (
        "peripherals/esp32c3/uart.rs",
        "UART0_BASE",
        "OWN BASE (esp32c3.yaml: uart0). Used by `default_source_id` to pick an \
         interrupt-matrix source when the descriptor names none — i.e. the \
         constant is a lookup KEY, not an address that is ever written. \
         Converting it means changing how the default is selected, not just \
         where the number comes from.",
    ),
    (
        "peripherals/esp32c3/uart.rs",
        "UART1_BASE",
        "OWN BASE (esp32c3.yaml: uart1). Same interrupt-source lookup key as \
         UART0_BASE above.",
    ),
    (
        "peripherals/esp32/dport.rs",
        "BASE",
        "OWN BASE (esp32.yaml: dport). Classic-ESP32 registration constant. \
         This is the family where apb_ctrl shadowed SYSCON, so its address map \
         should be moved wholesale and proven, not one constant at a time.",
    ),
    (
        "peripherals/esp32/efuse.rs",
        "BASE",
        "OWN BASE (esp32.yaml: efuse). Classic-ESP32 registration constant.",
    ),
    (
        "peripherals/esp32/i2c.rs",
        "I2C0_BASE",
        "OWN BASE (esp32.yaml: i2c0). Classic-ESP32 registration constant.",
    ),
    (
        "peripherals/esp32/ledc.rs",
        "BASE",
        "OWN BASE (esp32.yaml: ledc). Classic-ESP32 registration constant.",
    ),
    (
        "peripherals/esp32/mcpwm.rs",
        "BASE",
        "OWN BASE (esp32.yaml: mcpwm0). Classic-ESP32 registration constant.",
    ),
    (
        "peripherals/esp32/rtc_cntl.rs",
        "BASE",
        "OWN BASE (esp32.yaml: rtc_cntl). Classic-ESP32 registration constant.",
    ),
    (
        "peripherals/esp32/sar_adc.rs",
        "SENS_BASE",
        "OWN BASE (esp32.yaml: sens_sar_adc). Note esp32.yaml declares `rtcio` \
         as 4 KB from 0x3FF4_8400, which swallows this window whole — see the \
         Rule 2 allowlist entry.",
    ),
    (
        "peripherals/esp32/sha.rs",
        "BASE",
        "OWN BASE (esp32.yaml: sha). Classic-ESP32 registration constant.",
    ),
    (
        "peripherals/esp32/syscon.rs",
        "BASE",
        "OWN BASE (esp32.yaml: syscon). This is the exact peripheral apb_ctrl \
         shadowed. The base is now correct and Rule 2 below enforces that \
         nothing re-collides with it; the duplicated constant remains.",
    ),
    (
        "peripherals/esp32s3/ds.rs",
        "DS_BASE",
        "OWN BASE. Declared by esp32c3.yaml as `ds`; no esp32s3 YAML declares a \
         `ds` at all, so there is currently no S3 id to resolve it from. \
         Resolving it against the C3 entry would be the shared-register-map \
         mistake this repo has already paid for — C3 and S3 do not share \
         offsets by default. Left as-is and flagged.",
    ),
    (
        "peripherals/esp32s3/hmac.rs",
        "HMAC_BASE",
        "OWN BASE. Same C3-declared/S3-undeclared split as ds.rs above.",
    ),
    (
        "peripherals/esp32s3/i2c.rs",
        "I2C0_BASE",
        "OWN BASE (esp32s3.yaml: i2c0).",
    ),
    (
        "peripherals/esp32s3/i2c.rs",
        "I2C1_BASE",
        "NO YAML OWNER — no esp32s3 chip YAML declares an `i2c1`. The model is \
         reachable at an address the descriptor never mentions, which is the \
         same one-home defect pointing the other way. The fix is a YAML entry, \
         not a code change, and adding peripherals to a shipped chip map needs \
         its own review.",
    ),
    (
        "peripherals/esp32s3/i2s.rs",
        "I2S0_BASE",
        "NO YAML OWNER — esp32s3 YAMLs declare no `i2s0`. Same as I2C1_BASE.",
    ),
    (
        "peripherals/esp32s3/i2s.rs",
        "I2S1_BASE",
        "NO YAML OWNER on S3; the value matches esp32c3.yaml's `i2s0`. \
         Resolving S3 against a C3 entry is the shared-map trap. Flagged, not \
         converted.",
    ),
    (
        "peripherals/esp32s3/lcd_cam.rs",
        "LCD_CAM_BASE",
        "NO YAML OWNER — no chip YAML declares `lcd_cam`. Same as I2C1_BASE.",
    ),
    (
        "peripherals/esp32s3/ledc.rs",
        "LEDC_BASE",
        "OWN BASE; matches esp32c3.yaml `ledc`, no S3 YAML entry. Same \
         C3/S3 split as ds.rs.",
    ),
    (
        "peripherals/esp32s3/mcpwm.rs",
        "MCPWM0_BASE",
        "OWN BASE (esp32s3.yaml: mcpwm0).",
    ),
    (
        "peripherals/esp32s3/mcpwm.rs",
        "MCPWM1_BASE",
        "NO YAML OWNER — no chip YAML declares `mcpwm1`. Same as I2C1_BASE.",
    ),
    // ── memory-region prefixes, not peripheral bases ────────────────────────
    (
        "peripherals/esp32c3/wifi_mac.rs",
        "DRAM_BASE",
        "NOT A PERIPHERAL BASE. 0x3FC0_0000 is the C3 DRAM window, used to \
         translate DMA descriptor pointers. Memory regions live under a chip \
         YAML's `memory_regions`, not `peripherals`, so ChipMap cannot resolve \
         it. Caught only because the constant is named *_BASE. Kept listed \
         rather than name-mangled around the rule.",
    ),
    (
        "peripherals/esp32s3/gdma.rs",
        "DRAM_ADDR_PREFIX",
        "NOT A PERIPHERAL BASE. S3 DRAM window prefix for DMA descriptor \
         address translation, same as wifi_mac.rs DRAM_BASE.",
    ),
];

/// Rule 2 exemptions: chip YAMLs whose declared peripheral windows overlap.
///
/// An overlap is decided silently by registration order. Each of these is a
/// real ambiguity in a shipped memory map; they are recorded because changing a
/// shipped chip's address map is a behavioural change that needs its own
/// firmware proof, not a drive-by edit.
///
/// Format: `(chip yaml file name, peripheral id A, peripheral id B, why)`.
/// Ids are stored sorted so the entry does not depend on YAML ordering.
const WINDOW_OVERLAP_ALLOWLIST: &[(&str, &str, &str, &str)] = &[
    (
        "rp2040.yaml",
        "clk_rst",
        "io_bank0",
        "`clk_rst` is a deliberate catch-all covering CLOCKS/RESETS/PSM/XOSC/\
         PLL_SYS/PLL_USB as one 160 KB block (0x4000_8000..0x4003_0000), and \
         IO_BANK0 sits inside it at 0x4001_4000. It cannot simply be narrowed: \
         XOSC and both PLLs live ABOVE io_bank0 in that same span, so a single \
         shorter window would drop the registers Zephyr's clock bring-up polls. \
         The overlap is safe because bus routing is greatest-start-wins and \
         history-independent (see `overlapping_windows_route_history_\
         independently`), so io_bank0 deterministically owns its 4 KB whatever \
         the registration order — which is exactly what makes GPIOn_CTRL.FUNCSEL \
         reach a model that acts on it instead of being absorbed as inert \
         storage.",
    ),
    (
        "esp32.yaml",
        "rtcio",
        "sens_sar_adc",
        "`rtcio` is declared 4 KB from 0x3FF4_8400 but really owns only 0x400, \
         so its declared window swallows `sens_sar_adc` (0x3FF4_8800) whole. \
         Narrowing rtcio to its true extent is the fix; it changes what answers \
         in 0x3FF4_8800..0x3FF4_9400 on classic ESP32 and needs a firmware run.",
    ),
    (
        "esp32.yaml",
        "io_mux",
        "rtcio",
        "Same over-declared `rtcio` 4 KB window, overlapping `io_mux` at \
         0x3FF4_9000 for 0x400 bytes.",
    ),
    (
        "nrf54l15.yaml",
        "gpio1",
        "temp",
        "nRF54L15 uses documented negative-offset remaps (MDK base - 0x504) to \
         line up Zephyr's register views; the side effect is 0x504 bytes of \
         gpio1 landing inside `temp`'s window.",
    ),
    (
        "nrf54l15.yaml",
        "gpio0",
        "wdt31",
        "Same documented -0x504 remap, gpio0 into `wdt31`'s window.",
    ),
    (
        "stm32l476.yaml",
        "comp",
        "syscfg",
        "The classic STM32 0x4001_0000 block: SYSCFG/COMP/EXTI are one silicon \
         block declared as three 1 KB peripherals at 0x200 spacing, so each \
         declared window runs into the next. Correct fix is to declare their \
         true 0x200 extents.",
    ),
    (
        "stm32l476.yaml",
        "comp",
        "exti",
        "Same STM32 0x4001_0000 SYSCFG/COMP/EXTI block.",
    ),
];

// ─────────────────────────────────────────────────────────────────────────────
// Source scanning
// ─────────────────────────────────────────────────────────────────────────────

/// Anything at or above this is treated as an absolute MMIO address rather than
/// a register offset. Every peripheral window on every chip in `configs/chips/`
/// sits above it (lowest is the RP2040 XIP SSI at 0x1800_0000); every register
/// offset within a peripheral sits far below.
const MMIO_FLOOR: u64 = 0x1000_0000;

/// Rule 2 floor. Lower than [`MMIO_FLOOR`] because a real peripheral window can
/// legitimately sit low (some imported descriptors map peripherals from
/// 0x4000), but high enough to reject the non-MMIO bus-slot indices
/// (`base_address: 1`, `0xf`) used by the imported Renode-style platform files.
const MIN_MMIO_WINDOW_BASE: u64 = 0x1000;

fn core_src_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn configs_chips_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips")
}

fn rust_sources_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            rust_sources_under(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

fn yaml_files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            yaml_files_under(&p, out);
        } else if p.extension().is_some_and(|e| e == "yaml" || e == "yml") {
            out.push(p);
        }
    }
}

/// Blank out comments and string/char literals, preserving byte length and line
/// structure, so brace matching and literal scanning only ever see real code.
///
/// Without this the scanner miscounts: a `"{:#x}"` format string inside a test
/// module throws the brace depth off and leaks test code into the production
/// scan. That is not hypothetical — it produced two false positives on the
/// first run of this gate.
fn blank_noncode(src: &str) -> String {
    let b: Vec<char> = src.chars().collect();
    let mut out = b.clone();
    let n = b.len();
    let blank = |out: &mut Vec<char>, a: usize, z: usize| {
        for c in out.iter_mut().take(z.min(n)).skip(a) {
            if *c != '\n' {
                *c = ' ';
            }
        }
    };
    let mut i = 0usize;
    while i < n {
        let c = b[i];
        if c == '/' && i + 1 < n && b[i + 1] == '/' {
            let mut j = i;
            while j < n && b[j] != '\n' {
                j += 1;
            }
            blank(&mut out, i, j);
            i = j;
        } else if c == '/' && i + 1 < n && b[i + 1] == '*' {
            let mut depth = 1usize;
            let mut j = i + 2;
            while j < n && depth > 0 {
                if b[j] == '/' && j + 1 < n && b[j + 1] == '*' {
                    depth += 1;
                    j += 2;
                } else if b[j] == '*' && j + 1 < n && b[j + 1] == '/' {
                    depth -= 1;
                    j += 2;
                } else {
                    j += 1;
                }
            }
            blank(&mut out, i, j);
            i = j;
        } else if c == 'r' && i + 1 < n && (b[i + 1] == '#' || b[i + 1] == '"') {
            let mut k = i + 1;
            let mut hashes = 0usize;
            while k < n && b[k] == '#' {
                hashes += 1;
                k += 1;
            }
            if k < n && b[k] == '"' {
                // Find the closing `"` followed by `hashes` `#`.
                let mut j = k + 1;
                loop {
                    if j >= n {
                        break;
                    }
                    if b[j] == '"' {
                        let mut h = 0usize;
                        while h < hashes && j + 1 + h < n && b[j + 1 + h] == '#' {
                            h += 1;
                        }
                        if h == hashes {
                            j = j + 1 + hashes;
                            break;
                        }
                    }
                    j += 1;
                }
                blank(&mut out, i, j);
                i = j;
            } else {
                i += 1;
            }
        } else if c == '"' {
            let mut j = i + 1;
            while j < n {
                if b[j] == '\\' {
                    j += 2;
                    continue;
                }
                if b[j] == '"' {
                    j += 1;
                    break;
                }
                j += 1;
            }
            blank(&mut out, i, j);
            i = j;
        } else if c == '\'' {
            // A char literal is `'x'` or `'\x'`; anything else is a lifetime.
            let end = if i + 2 < n && b[i + 1] == '\\' {
                let mut j = i + 2;
                while j < n && b[j] != '\'' {
                    j += 1;
                }
                if j < n {
                    Some(j + 1)
                } else {
                    None
                }
            } else if i + 2 < n && b[i + 2] == '\'' {
                Some(i + 3)
            } else {
                None
            };
            match end {
                Some(j) => {
                    blank(&mut out, i, j);
                    i = j;
                }
                None => i += 1,
            }
        } else {
            i += 1;
        }
    }
    out.into_iter().collect()
}

/// Blank out `#[cfg(test)] mod NAME { ... }` bodies. Input must already be
/// [`blank_noncode`]'d so brace depth is trustworthy.
fn blank_test_modules(code: &str) -> String {
    let b: Vec<char> = code.chars().collect();
    let mut out = b.clone();
    let n = b.len();
    let s: String = code.to_string();
    let mut search_from = 0usize;
    while let Some(rel) = s[search_from..].find("#[cfg(test)]") {
        let start = search_from + rel;
        // Accept `#[cfg(test)]` followed by optional whitespace, optional
        // `pub`, `mod NAME {`.
        let tail = &s[start..];
        let Some(brace_rel) = tail.find('{') else {
            break;
        };
        let header = &tail[..brace_rel];
        if !header.contains("mod ") {
            search_from = start + "#[cfg(test)]".len();
            continue;
        }
        let open = start + brace_rel;
        let mut depth = 0usize;
        let mut j = open;
        while j < n {
            if b[j] == '{' {
                depth += 1;
            } else if b[j] == '}' {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            j += 1;
        }
        for c in out.iter_mut().take((j + 1).min(n)).skip(start) {
            if *c != '\n' {
                *c = ' ';
            }
        }
        search_from = (j + 1).min(n);
    }
    out.into_iter().collect()
}

/// One Rule 1 violation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct BaseConst {
    /// Path relative to `crates/core/src`.
    file: String,
    line: usize,
    name: String,
    value: u64,
}

/// Find `const|static NAME: uNN = 0x…;` where NAME mentions BASE or ADDR and
/// the literal is an absolute MMIO address.
fn scan_hardcoded_bases() -> Vec<BaseConst> {
    let root = core_src_dir();
    let scan_root = root.join("peripherals");
    let mut files = Vec::new();
    rust_sources_under(&scan_root, &mut files);

    let mut found = Vec::new();
    for path in files {
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let code = blank_test_modules(&blank_noncode(&raw));
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        for (idx, line) in code.lines().enumerate() {
            if let Some(hit) = parse_base_const(line) {
                let (name, value) = hit;
                if value >= MMIO_FLOOR && value <= u32::MAX as u64 {
                    found.push(BaseConst {
                        file: rel.clone(),
                        line: idx + 1,
                        name,
                        value,
                    });
                }
            }
        }
    }
    found.sort();
    found
}

/// Parse a single line for `const|static NAME: <int ty> = 0x…;`.
/// Returns `(name, value)` when NAME contains BASE or ADDR.
fn parse_base_const(line: &str) -> Option<(String, u64)> {
    let t = line.trim_start();
    let t = t.strip_prefix("pub ").unwrap_or(t).trim_start();
    // `pub(crate)` etc.
    let t = if t.starts_with("pub(") {
        let close = t.find(')')?;
        t[close + 1..].trim_start()
    } else {
        t
    };
    let rest = t
        .strip_prefix("const ")
        .or_else(|| t.strip_prefix("static "))?;
    let colon = rest.find(':')?;
    let name = rest[..colon].trim();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    let upper = name.to_ascii_uppercase();
    if !upper.contains("BASE") && !upper.contains("ADDR") {
        return None;
    }
    let after = &rest[colon + 1..];
    let eq = after.find('=')?;
    let ty = after[..eq].trim();
    if !matches!(ty, "u32" | "u64" | "usize") {
        return None;
    }
    let val = after[eq + 1..].trim().trim_end_matches(';').trim();
    let hex = val.strip_prefix("0x").or_else(|| val.strip_prefix("0X"))?;
    let digits: String = hex.chars().filter(|c| *c != '_').collect();
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(&digits, 16)
        .ok()
        .map(|v| (name.to_string(), v))
}

// ─────────────────────────────────────────────────────────────────────────────
// Chip YAML scanning
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Window {
    id: String,
    base: u64,
    size: Option<u64>,
}

/// `"4KB"`, `"1024"`, `0x1000` → bytes.
fn parse_size_str(s: &str) -> Option<u64> {
    let s = s.trim();
    let lower = s.to_ascii_lowercase();
    let (num, mult) = if let Some(p) = lower.strip_suffix("kb") {
        (p, 1024)
    } else if let Some(p) = lower.strip_suffix("mb") {
        (p, 1024 * 1024)
    } else if let Some(p) = lower.strip_suffix('k') {
        (p, 1024)
    } else if let Some(p) = lower.strip_suffix('m') {
        (p, 1024 * 1024)
    } else if let Some(p) = lower.strip_suffix('b') {
        (p, 1)
    } else {
        (lower.as_str(), 1)
    };
    let num = num.trim();
    let v = if let Some(h) = num.strip_prefix("0x") {
        u64::from_str_radix(h, 16).ok()?
    } else {
        num.parse::<u64>().ok()?
    };
    Some(v * mult)
}

fn value_to_u64(v: &serde_yaml::Value) -> Option<u64> {
    match v {
        serde_yaml::Value::Number(n) => n.as_u64(),
        serde_yaml::Value::String(s) => {
            let s = s.trim();
            if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                u64::from_str_radix(&h.replace('_', ""), 16).ok()
            } else {
                s.parse::<u64>().ok()
            }
        }
        _ => None,
    }
}

/// Every chip YAML's declared peripheral windows, keyed by file name.
fn chip_windows() -> Vec<(String, Vec<Window>)> {
    let dir = configs_chips_dir();
    let mut files = Vec::new();
    yaml_files_under(&dir, &mut files);
    let mut out = Vec::new();
    for path in files {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(doc) = serde_yaml::from_str::<serde_yaml::Value>(&text) else {
            continue;
        };
        let Some(list) = doc.get("peripherals").and_then(|p| p.as_sequence()) else {
            continue;
        };
        let mut wins = Vec::new();
        for item in list {
            let Some(id) = item.get("id").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(base) = item.get("base_address").and_then(value_to_u64) else {
                continue;
            };
            // Only system-bus MMIO windows can collide in the way this rule is
            // about. The 200-odd auto-imported descriptors under
            // `configs/chips/onboarding/` carry Renode-style entries where
            // `base_address` is a slot index on a non-MMIO parent bus
            // (`bus: pci`, `bus: usbEhci`, `base_address: 1`); those share a
            // "base" by construction and mean nothing here.
            if base < MIN_MMIO_WINDOW_BASE {
                continue;
            }
            if item
                .get("bus")
                .and_then(|v| v.as_str())
                .is_some_and(|b| b != "sysbus")
            {
                continue;
            }
            let size = item.get("size").and_then(|v| match v {
                serde_yaml::Value::String(s) => parse_size_str(s),
                other => value_to_u64(other),
            });
            wins.push(Window {
                id: id.to_string(),
                base,
                size,
            });
        }
        // Key by path relative to configs/chips so `onboarding/nrf52840.yaml`
        // stays distinguishable from `nrf52840.yaml`.
        let rel = path
            .strip_prefix(&dir)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        out.push((rel, wins));
    }
    out
}

/// A Rule 2 violation: two peripherals in one chip YAML whose windows collide.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Collision {
    chip: String,
    a: String,
    b: String,
    same_base: bool,
    detail: String,
}

fn scan_chip_collisions() -> Vec<Collision> {
    let mut out = Vec::new();
    for (chip, wins) in chip_windows() {
        for i in 0..wins.len() {
            for j in (i + 1)..wins.len() {
                let (x, y) = (&wins[i], &wins[j]);
                let same_base = x.base == y.base;
                let overlap = match (x.size, y.size) {
                    (Some(sx), Some(sy)) => x.base < y.base + sy && y.base < x.base + sx,
                    // A window with no declared size cannot be shown to
                    // overlap; only an exact base tie is provable.
                    _ => same_base,
                };
                if !overlap {
                    continue;
                }
                let (a, b) = if x.id <= y.id { (x, y) } else { (y, x) };
                let detail = format!(
                    "{} @ {:#010x} (size {}) vs {} @ {:#010x} (size {})",
                    a.id,
                    a.base,
                    a.size.map(|s| format!("{s:#x}")).unwrap_or("?".into()),
                    b.id,
                    b.base,
                    b.size.map(|s| format!("{s:#x}")).unwrap_or("?".into()),
                );
                out.push(Collision {
                    chip: chip.clone(),
                    a: a.id.clone(),
                    b: b.id.clone(),
                    same_base,
                    detail,
                });
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// The gates
// ─────────────────────────────────────────────────────────────────────────────

/// Rule 1: no production peripheral model may hardcode an absolute base.
#[test]
fn rule1_no_hardcoded_peripheral_base_in_models() {
    let found = scan_hardcoded_bases();
    let allowed: BTreeSet<(&str, &str)> = HARDCODED_BASE_ALLOWLIST
        .iter()
        .map(|(f, n, _)| (*f, *n))
        .collect();

    let mut violations = Vec::new();
    for hit in &found {
        if !allowed.contains(&(hit.file.as_str(), hit.name.as_str())) {
            violations.push(format!(
                "  {}:{}  `{}` = {:#010x}",
                hit.file, hit.line, hit.name, hit.value
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "\n\nA peripheral model hardcodes an absolute MMIO base address.\n\
         A base address belongs to the chip YAML; a copy in Rust is a second \
         home for the same fact with nothing forcing the two to agree, and the \
         disagreement fails SILENTLY (a wrong address is still a valid address \
         — it lands in another peripheral's window and is swallowed).\n\n\
         Read the base from the chip descriptor instead:\n\
           * the model's OWN base  → `p_cfg.base_address` in the family factory\n\
           * a SIBLING's base      → `peripherals::chip_map::ChipMap::base_of(id)`\n\n\
         Register offsets WITHIN a peripheral are fine and are not checked.\n\n\
         New hardcoded bases ({}):\n{}\n\n\
         If it genuinely cannot be converted, add it to HARDCODED_BASE_ALLOWLIST \
         in crates/core/src/tests/yaml_owned_base_contract.rs with a real \
         reason.\n",
        violations.len(),
        violations.join("\n")
    );
}

/// Rule 1, shrink-only: an allowlist entry that no longer violates must be
/// deleted, so a converted model cannot leave a stale exemption behind for the
/// next offender to hide under.
#[test]
fn rule1_allowlist_shrinks_only() {
    let found = scan_hardcoded_bases();
    let live: BTreeSet<(&str, &str)> = found
        .iter()
        .map(|h| (h.file.as_str(), h.name.as_str()))
        .collect();

    let mut stale = Vec::new();
    for (file, name, _) in HARDCODED_BASE_ALLOWLIST {
        if !live.contains(&(*file, *name)) {
            stale.push(format!("  {file} :: {name}"));
        }
    }

    assert!(
        stale.is_empty(),
        "\n\nStale HARDCODED_BASE_ALLOWLIST entries — these no longer hardcode a \
         base, so their exemption must be deleted.\n\
         An allowlist that only grows stops being a ratchet.\n\n{}\n",
        stale.join("\n")
    );
}

/// Rule 1 self-check: the scanner must see production code and must NOT see
/// `#[cfg(test)]` modules. A scanner that silently reads nothing would make
/// Rule 1 vacuously green forever.
#[test]
fn rule1_scanner_is_not_vacuous() {
    let root = core_src_dir().join("peripherals");
    let mut files = Vec::new();
    rust_sources_under(&root, &mut files);
    assert!(
        files.len() > 100,
        "expected the peripherals tree to hold >100 .rs files, found {} — \
         the scanner is looking in the wrong place and Rule 1 is vacuous",
        files.len()
    );

    // The allowlist is non-empty, so the scanner must be finding those.
    let found = scan_hardcoded_bases();
    assert!(
        !found.is_empty(),
        "the scanner found zero base constants while HARDCODED_BASE_ALLOWLIST \
         lists {} — the scan is broken and Rule 1 is vacuous",
        HARDCODED_BASE_ALLOWLIST.len()
    );

    // Test-module constants must be invisible. `nrf52/gpiote.rs` keeps
    // `T_GPIO0_BASE`/`T_GPIO1_BASE` inside `#[cfg(test)] mod tests`; if the
    // brace matcher regresses (e.g. miscounting braces inside a format string)
    // they leak into the production scan and this catches it.
    let leaked: Vec<_> = found
        .iter()
        .filter(|h| h.name.starts_with("T_GPIO"))
        .collect();
    assert!(
        leaked.is_empty(),
        "test-module constants leaked into the production scan: {leaked:?}"
    );
}

/// Rule 2: a chip YAML may not disagree with itself about its own memory map.
#[test]
fn rule2_chip_yaml_peripheral_windows_do_not_collide() {
    let found = scan_chip_collisions();
    let allowed: BTreeSet<(&str, &str, &str)> = WINDOW_OVERLAP_ALLOWLIST
        .iter()
        .map(|(c, a, b, _)| (*c, *a, *b))
        .collect();

    let mut violations = Vec::new();
    for c in &found {
        // Allowlist keys on the bare file name so `onboarding/x.yaml` and
        // `x.yaml` both match a single entry only if they genuinely share the
        // defect; the file name is carried in the message either way.
        let leaf = c.chip.rsplit('/').next().unwrap_or(&c.chip);
        if !allowed.contains(&(leaf, c.a.as_str(), c.b.as_str())) {
            violations.push(format!(
                "  {} :: {}{}",
                c.chip,
                c.detail,
                if c.same_base {
                    "  [IDENTICAL BASE]"
                } else {
                    ""
                }
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "\n\nA chip YAML declares two peripherals whose MMIO windows collide.\n\
         Bus routing ties are resolved by REGISTRATION ORDER, silently. This is \
         how classic-ESP32 `apb_ctrl` shadowed SYSCON: SYSCLK_CONF read \
         0xFFFF_FFFF, getApbFrequency() returned 78125 Hz, and \"Serial doesn't \
         work on classic ESP32\" was believed for about a year.\n\n\
         Fix the chip YAML: give each peripheral a base and a `size:` that \
         describe the window it actually owns.\n\n\
         New collisions ({}):\n{}\n\n\
         If a shipped map genuinely cannot change yet, add it to \
         WINDOW_OVERLAP_ALLOWLIST in \
         crates/core/src/tests/yaml_owned_base_contract.rs with a real reason.\n",
        violations.len(),
        violations.join("\n")
    );
}

/// Rule 2, shrink-only.
#[test]
fn rule2_allowlist_shrinks_only() {
    let found = scan_chip_collisions();
    let live: BTreeSet<(String, String, String)> = found
        .iter()
        .map(|c| {
            (
                c.chip.rsplit('/').next().unwrap_or(&c.chip).to_string(),
                c.a.clone(),
                c.b.clone(),
            )
        })
        .collect();

    let mut stale = Vec::new();
    for (chip, a, b, _) in WINDOW_OVERLAP_ALLOWLIST {
        if !live.contains(&(chip.to_string(), a.to_string(), b.to_string())) {
            stale.push(format!("  {chip} :: {a} vs {b}"));
        }
    }

    assert!(
        stale.is_empty(),
        "\n\nStale WINDOW_OVERLAP_ALLOWLIST entries — these chip YAMLs no longer \
         collide, so their exemption must be deleted.\n\n{}\n",
        stale.join("\n")
    );
}

/// Rule 2 self-check: the YAML scanner must actually be reading chip files.
#[test]
fn rule2_scanner_is_not_vacuous() {
    let windows = chip_windows();
    assert!(
        windows.len() >= 20,
        "expected >=20 chip YAMLs under configs/chips, found {} — \
         Rule 2 is vacuous",
        windows.len()
    );
    let total: usize = windows.iter().map(|(_, w)| w.len()).sum();
    assert!(
        total > 400,
        "expected >400 declared peripheral windows across all chips, found \
         {total} — Rule 2 is vacuous"
    );
    // nRF52840 is the chip the GPIOTE defect lived on; its remapped gpio1 must
    // be visible to the scanner, otherwise the fix's premise is unverified.
    let (_, nrf) = windows
        .iter()
        .find(|(f, _)| f == "nrf52840.yaml")
        .expect("configs/chips/nrf52840.yaml must be scanned");
    let gpio1 = nrf
        .iter()
        .find(|w| w.id == "gpio1")
        .expect("nrf52840.yaml must declare gpio1");
    assert_eq!(
        gpio1.base, 0x5000_1000,
        "nrf52840.yaml gpio1 is remapped off Nordic's silicon P1 base; if this \
         ever changes, Nrf52Gpiote follows it via ChipMap and this contract's \
         premise needs revisiting"
    );
}

#[cfg(test)]
mod scanner_unit_tests {
    use super::*;

    #[test]
    fn parses_a_base_const() {
        assert_eq!(
            parse_base_const("const GPIO1_BASE: u32 = 0x5000_0300;"),
            Some(("GPIO1_BASE".to_string(), 0x5000_0300))
        );
        assert_eq!(
            parse_base_const("pub const BT_BASE: u64 = 0x6003_1000;"),
            Some(("BT_BASE".to_string(), 0x6003_1000))
        );
    }

    #[test]
    fn ignores_offsets_and_masks() {
        // Not BASE/ADDR-named → not a base claim.
        assert_eq!(parse_base_const("const OFF_CONFIG_0: u64 = 0x510;"), None);
        assert_eq!(
            parse_base_const("const CONFIG_WRITE_MASK: u32 = 0x0013_3F03;"),
            None
        );
        // BASE-named but below the MMIO floor — filtered by the caller, so the
        // parser still returns it; assert the value so the floor is testable.
        assert_eq!(
            parse_base_const("const GPIO_IN_BASE: u32 = 0x510;"),
            Some(("GPIO_IN_BASE".to_string(), 0x510))
        );
        const { assert!(0x510 < MMIO_FLOOR) };
    }

    #[test]
    fn blanks_braces_inside_string_literals() {
        // The exact shape that broke the first version of this scanner.
        let src = "#[cfg(test)]\nmod tests {\n let s = \"{\";\n const X_BASE: u32 = 0x6000_0000;\n}\nconst Y_BASE: u32 = 0x6001_0000;\n";
        let code = blank_test_modules(&blank_noncode(src));
        assert!(
            !code.contains("X_BASE"),
            "test-module const leaked: {code:?}"
        );
        assert!(
            code.contains("Y_BASE"),
            "production const was eaten: {code:?}"
        );
    }

    #[test]
    fn parses_sizes() {
        assert_eq!(parse_size_str("4KB"), Some(4096));
        assert_eq!(parse_size_str("0x1000"), Some(4096));
        assert_eq!(parse_size_str("256"), Some(256));
        assert_eq!(parse_size_str("1MB"), Some(1024 * 1024));
    }
}
