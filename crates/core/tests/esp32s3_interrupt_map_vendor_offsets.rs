// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! The ESP32-S3 interrupt matrix descriptor must carry the **S3's** register
//! map, and the ESP32-C3's must stay on the **C3's**.
//!
//! `configs/peripherals/esp32s3/interrupt_core0.yaml` shipped a hybrid of the
//! two parts. The C3 has no `GPIO_INTERRUPT_APP_MAP` (0x48), no
//! `GPIO_INTERRUPT_APP_NMI_MAP` (0x4C) and no `SPI_INTR_1_MAP` (0x50); those
//! three rows were missing from the S3 file, so every register after them sat
//! 8, then 20, then 40 bytes low — `SPI_INTR_2_MAP` at 0x4C instead of 0x54,
//! `UART_INTR_MAP` at 0x58 instead of 0x6C, `TG_T0_INT_MAP` at 0xA0 instead of
//! 0xC8. The first register was named `MAC_INTR_MAP`, which is what the C3
//! calls it; on the S3 it is `PRO_MAC_INTR_MAP`, and *because the names
//! differed nothing ever compared the two files*. The 0x104..0x194 block held
//! the C3's RISC-V INTC control registers (`CPU_INT_ENABLE`, `CPU_INT_PRI_n`,
//! `CPU_INT_THRESH`), which the Xtensa LX7 S3 does not have at all.
//!
//! Why this needs a gate at all: an undecoded offset in a declarative
//! peripheral is a **silent no-op** — the write is dropped and the read
//! fabricates a zero (see `GenericPeripheral::read`/`write`). "Nothing happens"
//! is exactly what an untested fix looks like too, so the map is asserted here
//! against the vendored SVD rather than against a hand-copied table.
//!
//! Two chips, deliberately. A test that only proves the S3 now decodes at
//! 0x6C cannot tell a fixed S3 from a C3 that was dragged onto the S3's
//! offsets — and the C3 map is correct today (103/103 against its own SVD).
//! `bus/routing.rs` finds the C3 INTC by NAME AND BASE
//! (`p.name == "interrupt_core0" && p.base == 0x600C_2000`), which is exactly
//! what the S3 chip YAML also declares, so the two files are one edit away
//! from each other at all times.

use std::collections::HashMap;
use std::path::PathBuf;

use labwired_config::PeripheralDescriptor;
use labwired_core::peripherals::declarative::GenericPeripheral;
use labwired_core::Peripheral;

const S3_DESCRIPTOR: &str = "configs/peripherals/esp32s3/interrupt_core0.yaml";
const C3_DESCRIPTOR: &str = "configs/peripherals/esp32c3/interrupt_core0.yaml";
const S3_SVD: &str = "tests/fixtures/svd/esp32s3.svd";
const C3_SVD: &str = "tests/fixtures/real_world/esp32c3.svd";

fn root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn descriptor(rel: &str) -> PeripheralDescriptor {
    PeripheralDescriptor::from_file(root(rel)).unwrap_or_else(|e| panic!("load {rel}: {e}"))
}

/// The vendor's `INTERRUPT_CORE0` register map: name → (offset, reset value).
/// Parsed with the same `svd_ingestor` the CLI importer and the coverage scan
/// use, so this gate reads the SVD the way the rest of the repo does.
fn vendor_map(svd_rel: &str) -> HashMap<String, (u64, u32)> {
    let xml =
        std::fs::read_to_string(root(svd_rel)).unwrap_or_else(|e| panic!("read {svd_rel}: {e}"));
    let device = svd_ingestor::parse_svd(&xml).unwrap_or_else(|e| panic!("parse {svd_rel}: {e}"));
    let peripheral = device
        .peripherals
        .iter()
        .find(|p| p.name == "INTERRUPT_CORE0")
        .unwrap_or_else(|| panic!("{svd_rel} has no INTERRUPT_CORE0 peripheral"));
    let desc = svd_ingestor::process_peripheral(&device, peripheral)
        .unwrap_or_else(|e| panic!("process INTERRUPT_CORE0 from {svd_rel}: {e}"));
    let map: HashMap<String, (u64, u32)> = desc
        .registers
        .iter()
        .map(|r| (r.id.clone(), (r.address_offset, r.reset_value)))
        .collect();
    assert!(
        map.len() > 90,
        "{svd_rel} yielded only {} INTERRUPT_CORE0 registers — the SVD parse has \
         broken and every comparison below would pass vacuously",
        map.len()
    );
    map
}

/// Every register a descriptor declares, checked against the vendor's own map
/// for that silicon: same name, same offset, same reset value.
fn assert_conforms(descriptor_rel: &str, svd_rel: &str, min_registers: usize) {
    let desc = descriptor(descriptor_rel);
    let vendor = vendor_map(svd_rel);

    assert!(
        desc.registers.len() >= min_registers,
        "{descriptor_rel} declares only {} registers (expected at least \
         {min_registers}) — the descriptor was gutted, not fixed",
        desc.registers.len()
    );

    let mut findings = Vec::new();
    for reg in &desc.registers {
        match vendor.get(&reg.id) {
            None => findings.push(format!(
                "{}: not a register of this silicon's INTERRUPT_CORE0 \
                 (offset {:#05X} in our model)",
                reg.id, reg.address_offset
            )),
            Some(&(offset, reset)) => {
                if reg.address_offset != offset {
                    findings.push(format!(
                        "{}: modelled at {:#05X}, silicon puts it at {:#05X}",
                        reg.id, reg.address_offset, offset
                    ));
                }
                if reg.reset_value != reset {
                    findings.push(format!(
                        "{}: reset {:#010X}, silicon resets to {:#010X}",
                        reg.id, reg.reset_value, reset
                    ));
                }
            }
        }
    }

    assert!(
        findings.is_empty(),
        "{descriptor_rel} disagrees with {svd_rel} ({} findings of {} registers \
         checked):\n  {}",
        findings.len(),
        desc.registers.len(),
        findings.join("\n  ")
    );
}

/// Read a 32-bit register out of a declarative peripheral the way the bus does.
fn read32(p: &GenericPeripheral, offset: u64) -> u32 {
    p.read_u32(offset).expect("read_u32")
}

fn write32(p: &mut GenericPeripheral, offset: u64, value: u32) {
    p.write_u32(offset, value).expect("write_u32");
}

fn build(rel: &str) -> GenericPeripheral {
    GenericPeripheral::new(descriptor(rel))
}

/// Offset → register id, for saying *which* register answers an address.
fn register_at(rel: &str, offset: u64) -> Option<String> {
    descriptor(rel)
        .registers
        .iter()
        .find(|r| r.address_offset == offset)
        .map(|r| r.id.clone())
}

// ── ESP32-S3: the fix ────────────────────────────────────────────────────────

#[test]
fn esp32s3_interrupt_map_matches_the_vendor_svd() {
    assert_conforms(S3_DESCRIPTOR, S3_SVD, 30);

    // The four offsets the defect was found on, stated outright so a future
    // reader does not have to re-derive them from the SVD. Left column is what
    // the file used to say.
    assert_eq!(
        register_at(S3_DESCRIPTOR, 0x000).as_deref(),
        Some("PRO_MAC_INTR_MAP"),
        "source 0 is PRO_MAC_INTR_MAP on the S3; MAC_INTR_MAP is the C3's name \
         for it, and a name mismatch is what kept this out of every comparison"
    );
    assert_eq!(
        register_at(S3_DESCRIPTOR, 0x054).as_deref(),
        Some("SPI_INTR_2_MAP"),
        "SPI_INTR_2_MAP is at 0x54 on the S3 (it was modelled at the C3's 0x4C)"
    );
    assert_eq!(
        register_at(S3_DESCRIPTOR, 0x06C).as_deref(),
        Some("UART_INTR_MAP"),
        "UART_INTR_MAP is at 0x6C on the S3 (it was modelled at 0x58)"
    );
    assert_eq!(
        register_at(S3_DESCRIPTOR, 0x0C8).as_deref(),
        Some("TG_T0_INT_MAP"),
        "TG_T0_INT_MAP is at 0xC8 on the S3 (it was modelled at 0xA0)"
    );

    // The three S3-only rows whose absence produced the 8 → 20 → 40 byte drift.
    for (offset, id) in [
        (0x048u64, "GPIO_INTERRUPT_APP_MAP"),
        (0x04C, "GPIO_INTERRUPT_APP_NMI_MAP"),
        (0x050, "SPI_INTR_1_MAP"),
    ] {
        assert_eq!(
            register_at(S3_DESCRIPTOR, offset).as_deref(),
            Some(id),
            "{id} at {offset:#05X} is what the C3 lacks and the S3 has; without \
             it every register below is back on a C3 offset"
        );
    }

    // The C3's RISC-V INTC block has no counterpart on Xtensa. Its former
    // offsets here are ordinary source-map registers on this silicon.
    let s3 = descriptor(S3_DESCRIPTOR);
    for c3_only in [
        "CPU_INT_ENABLE",
        "CPU_INT_TYPE",
        "CPU_INT_CLEAR",
        "CPU_INT_EIP_STATUS",
        "CPU_INT_PRI_0",
        "CPU_INT_THRESH",
        "INTR_STATUS_REG_0",
        "MAC_INTR_MAP",
    ] {
        assert!(
            !s3.registers.iter().any(|r| r.id == c3_only),
            "{c3_only} is an ESP32-C3 register; the S3 model must not declare it"
        );
    }
}

#[test]
fn esp32s3_interrupt_matrix_decodes_at_the_vendor_offset() {
    let mut s3 = build(S3_DESCRIPTOR);

    // Reset state first: every *_MAP register on this part resets to 0x10, not
    // 0. A read of 0 here would mean either the wrong reset value or — worse —
    // that 0x6C decodes to nothing and the peripheral is fabricating a zero.
    assert_eq!(
        read32(&s3, 0x06C),
        0x10,
        "UART_INTR_MAP (0x6C) must reset to 0x10; 0 means the offset is \
         undecoded (silent no-op) or the reset value is still the old 0"
    );

    // Write-then-read at the vendor's offset. On the shipped model 0x6C fell in
    // the hole between UART2_INTR_MAP (0x60) and LEDC_INT_MAP (0x68 + 4), so
    // this write was dropped and the read returned 0.
    write32(&mut s3, 0x06C, 0x17);
    assert_eq!(
        read32(&s3, 0x06C),
        0x17,
        "a write to UART_INTR_MAP at the vendor's 0x6C must round-trip"
    );

    // …and it must be that register alone. 0x58 is where UART_INTR_MAP used to
    // sit; it is SPI_INTR_3_MAP on real S3 silicon and must not have moved.
    assert_eq!(
        register_at(S3_DESCRIPTOR, 0x058).as_deref(),
        Some("SPI_INTR_3_MAP")
    );
    assert_eq!(
        read32(&s3, 0x058),
        0x10,
        "the old (C3) UART offset must be untouched by a write to the real one"
    );

    // The Tier-1 S3 fixture binds a source through `INTMATRIX_BASE + 4*source`
    // (examples/tier1-fixture/esp32s3/src/main.rs, check_irq, source 42), so
    // that row must decode — it did before this change too, as the wrong
    // register.
    assert_eq!(
        register_at(S3_DESCRIPTOR, 4 * 42).as_deref(),
        Some("I2C_EXT0_INTR_MAP"),
        "source 42 is I2C_EXT0 on the S3"
    );
    write32(&mut s3, 4 * 42, 17);
    assert_eq!(read32(&s3, 4 * 42), 17);

    // The source-assertion words sit where the behavioural model puts them
    // (INTR_STATUS_BASE = 0x18C in peripherals/esp32s3/intmatrix.rs). Two
    // models of one block that disagree on an address is the defect this file
    // exists to prevent.
    assert_eq!(
        register_at(S3_DESCRIPTOR, 0x18C).as_deref(),
        Some("PRO_INTR_STATUS_0")
    );
    assert_eq!(
        register_at(S3_DESCRIPTOR, 0x19C).as_deref(),
        Some("CLOCK_GATE"),
        "CLOCK_GATE is at 0x19C on the S3 (it was modelled at the C3-shifted 0x1A0)"
    );
    assert_eq!(read32(&s3, 0x19C), 1, "CLOCK_GATE resets to 1");
}

// ── ESP32-C3: the chip that must not move ────────────────────────────────────

#[test]
fn esp32c3_interrupt_map_is_unchanged_at_its_own_offsets() {
    // The C3 descriptor is SVD-generated and fully conformant; assert all of it,
    // so "fixing" the S3 by dragging the C3 along cannot pass.
    assert_conforms(C3_DESCRIPTOR, C3_SVD, 100);

    let mut c3 = build(C3_DESCRIPTOR);

    // The C3's own UART_INTR_MAP is at 0x54 — NOT at the S3's 0x6C, which on
    // this part is RTC_CORE_INTR_MAP.
    assert_eq!(
        register_at(C3_DESCRIPTOR, 0x054).as_deref(),
        Some("UART_INTR_MAP")
    );
    assert_eq!(
        register_at(C3_DESCRIPTOR, 0x06C).as_deref(),
        Some("RTC_CORE_INTR_MAP"),
        "0x6C is UART_INTR_MAP on the S3 and RTC_CORE_INTR_MAP on the C3; if \
         these two ever agree, one of them is wrong"
    );
    write32(&mut c3, 0x054, 0x0C);
    assert_eq!(
        read32(&c3, 0x054),
        0x0C,
        "the C3's UART_INTR_MAP must still round-trip at its own offset"
    );

    // `SystemBus::rebuild_esp32c3_irq_cache` reads these exact offsets out of
    // this descriptor (bus/routing.rs): CPU_INT_ENABLE 0x104, per-line priority
    // 0x114 + line*4, CPU_INT_THRESH 0x194. They are C3-only registers and the
    // C3 INTC stops working without them.
    assert_eq!(
        register_at(C3_DESCRIPTOR, 0x104).as_deref(),
        Some("CPU_INT_ENABLE")
    );
    assert_eq!(
        register_at(C3_DESCRIPTOR, 0x114).as_deref(),
        Some("CPU_INT_PRI_0")
    );
    assert_eq!(
        register_at(C3_DESCRIPTOR, 0x194).as_deref(),
        Some("CPU_INT_THRESH")
    );
    write32(&mut c3, 0x104, 0xDEAD_BEEF);
    assert_eq!(read32(&c3, 0x104), 0xDEAD_BEEF);

    // Source 0 is MAC_INTR_MAP on the C3 and PRO_MAC_INTR_MAP on the S3.
    assert_eq!(
        register_at(C3_DESCRIPTOR, 0x000).as_deref(),
        Some("MAC_INTR_MAP")
    );
}
