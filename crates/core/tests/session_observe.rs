// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `Session`'s observe and stimulus surface: symbols and memory, decoded bus
//! frames, board-IO pins, input channels, logic capture, inspect, and
//! snapshot/restore.
//!
//! Two fixtures. The ARM CI fixture (`common::arm_fixture`, skips when not
//! built) is the minimal Cortex-M image. The nRF54L15 smart-ring probe
//! (`common::smart_ring_fixture`, a committed ELF, never skips) is the one with
//! real bus traffic: it reads the WHO_AM_I of four I²C device models on TWIM21
//! at cycles ~933, ~1725, ~2507 and ~3318, and its board carries two
//! `board_io` buttons on GPIO1.

mod common;

use labwired_core::bus::bus_trace::{BusPayload, I2cSym};
use labwired_core::session::{AddrOrSymbol, OpenOptions, Session, SessionError};
use labwired_core::sim_input::SimInputError;
use labwired_core::system::builder::*;
use std::time::Duration;

fn open_fixture(f: &common::Fixture) -> Session {
    let blobs = BlobMap::new();
    Session::open(
        BuildRequest {
            chip: &f.chip,
            system: &f.manifest,
            firmware: FirmwareSource::Elf(&f.fw),
            boot: BootMode::FastBoot,
            blobs: &blobs,
            options: BuildOptions::default(),
        },
        OpenOptions::default(),
    )
    .unwrap()
}

fn open_arm() -> Option<(Session, common::Fixture)> {
    let (fw, chip, manifest, expected) = common::arm_fixture()?;
    let f = common::Fixture {
        fw,
        chip,
        manifest,
        expected,
    };
    Some((open_fixture(&f), f))
}

fn open_ring() -> Session {
    open_fixture(&common::smart_ring_fixture())
}

/// The smart ring with `charger_detect` removed, so `touch` is the only
/// device exposing the `pressed` channel and a bare `set_input("pressed")`
/// is unambiguous.
fn open_ring_one_button() -> Session {
    let mut f = common::smart_ring_fixture();
    f.manifest.board_io.retain(|b| b.id != "charger_detect");
    open_fixture(&f)
}

#[test]
fn symbol_and_read_u32_resolve_the_vector_table() {
    let Some((s, f)) = open_arm() else {
        return;
    };
    let flash = f.chip.flash.base;
    let sp = s.read_u32(AddrOrSymbol::Addr(flash)).unwrap();
    let ram = f.chip.ram.base..=f.chip.ram.base + f.chip.ram.size;
    assert!(
        ram.contains(&u64::from(sp)),
        "initial SP must be in RAM {ram:#x?}, got {sp:#x}"
    );

    let vector = s.read_u32(AddrOrSymbol::Addr(flash + 4)).unwrap();
    assert!((flash..flash + f.chip.flash.size).contains(&u64::from(vector)));

    // What the session reads back is what the loader parses out of the file.
    let image = labwired_loader::load_elf_bytes(&f.fw).unwrap();
    let head = image
        .segments
        .iter()
        .find(|seg| seg.start_addr == flash)
        .expect("a segment at the flash base");
    assert_eq!(s.read_memory(flash, 8).unwrap(), head.data[..8]);

    // Core resolves symbols with its own ELF reader; the loader's
    // `resolve_symbol_in_elf` is an independent implementation over a
    // different parser. They must agree, Thumb bit included.
    let reset = s.symbol("Reset").expect("fixture ELF exports Reset");
    assert_eq!(
        labwired_loader::resolve_symbol_in_elf(&f.fw, "Reset"),
        Some(reset as u32)
    );
    assert_eq!(reset & 1, 1, "a Thumb function symbol keeps bit 0");

    // Reading through the symbol reads the function's first word, at the
    // address with the Thumb bit cleared.
    assert_eq!(
        s.read_u32(AddrOrSymbol::Symbol("Reset")).unwrap(),
        s.read_u32(AddrOrSymbol::Addr(reset & !1)).unwrap()
    );

    let bytes = s.read_memory(flash, 8).unwrap();
    assert_eq!(&bytes[0..4], &sp.to_le_bytes());
    assert_eq!(&bytes[4..8], &vector.to_le_bytes());

    assert!(s.symbol("no_such_symbol").is_none());
    assert!(matches!(
        s.read_u32(AddrOrSymbol::Symbol("no_such_symbol")),
        Err(SessionError::UnknownSymbol(name)) if name == "no_such_symbol"
    ));
}

#[test]
fn read_memory_of_unmapped_space_is_an_error_not_zeros() {
    let s = open_ring();
    assert!(matches!(
        s.read_memory(0xF000_0000, 4),
        Err(SessionError::Sim(_))
    ));
}

#[test]
fn frames_cursor_never_returns_an_event_twice() {
    let mut s = open_ring();
    // Up to cycle 2000 the probe has read the BMI270 and the MAX30102; the
    // TMP117 and DRV2605 transactions come after.
    s.run_cycles(2_000).unwrap();
    let a = s.frames();
    s.run_for(Duration::from_millis(50)).unwrap();
    let b = s.frames();
    let c = s.frames();

    let i2c_addrs = |frames: &[labwired_core::session::Frame]| -> Vec<u8> {
        frames
            .iter()
            .filter(|f| f.bus == "twi21")
            .filter_map(|f| match f.payload {
                BusPayload::I2c {
                    kind: I2cSym::AddrWrite,
                    byte,
                    ..
                } => Some(byte >> 1),
                _ => None,
            })
            .collect()
    };
    assert!(!a.is_empty(), "the I2C fixture must produce bus traffic");
    assert_eq!(i2c_addrs(&a), vec![0x68, 0x57], "BMI270, MAX30102");
    assert_eq!(i2c_addrs(&b), vec![0x48, 0x5A], "TMP117, DRV2605");
    assert!(
        c.is_empty(),
        "nothing new happened between the last two reads"
    );

    assert!(b.iter().all(|f| a.iter().all(|g| g.seq != f.seq)));
    // Contiguous: the second read starts exactly where the first stopped.
    let seqs: Vec<u64> = a.iter().chain(&b).map(|f| f.seq).collect();
    assert!(seqs.windows(2).all(|w| w[1] == w[0] + 1), "{seqs:?}");

    let hz = s.cpu_hz();
    for f in a.iter().chain(&b) {
        assert!(!f.summary.is_empty());
        assert_eq!(
            f.at,
            Duration::from_nanos(f.cycle * 1_000_000_000 / hz),
            "frame time is its cycle at the session clock"
        );
    }
    let first_addr = a
        .iter()
        .find(|f| matches!(f.payload, BusPayload::I2c { .. }))
        .unwrap();
    assert_eq!(first_addr.summary, "addr 0x68 W ack");
}

#[test]
fn set_pin_drives_a_board_io_input_and_logic_captures_the_edges() {
    let mut s = open_ring();
    // `touch` is gpio1 pin 13, active high: released reads low.
    let initial = s.watch_logic(&[("gpio1", 13)]).unwrap();
    assert_eq!(initial, vec![Some(false)]);

    s.run_cycles(1_000).unwrap();
    let pressed_at = s.cycles();
    s.set_pin("touch", true).unwrap();
    s.run_cycles(1_000).unwrap();
    let released_at = s.cycles();
    s.set_pin("touch", false).unwrap();
    s.run_cycles(1_000).unwrap();

    // An edge is observed at the first engine-cycle boundary after the level
    // changed (see `logic_capture`'s observation semantics), so a level set
    // while paused at cycle `c` is stamped `c + 1`.
    let batch = s.logic(0);
    let edges: Vec<(u32, u64, bool)> = batch
        .edges
        .iter()
        .map(|e| (e.ch, e.cycle, e.value))
        .collect();
    assert_eq!(
        edges,
        vec![(0, pressed_at + 1, true), (0, released_at + 1, false)],
        "one edge per level change, at the boundary after it"
    );
    assert!(
        s.logic(batch.cursor).edges.is_empty(),
        "the cursor acknowledges"
    );

    assert!(matches!(
        s.set_pin("no_such_binding", true),
        Err(SessionError::UnknownPin(id)) if id == "no_such_binding"
    ));
    // An output binding is not something a stimulus can drive.
    assert!(matches!(
        s.set_pin("motor_enable", true),
        Err(SessionError::UnknownPin(_))
    ));
    assert!(s.watch_logic(&[("gpio9", 0)]).is_err());
}

#[test]
fn set_input_reaches_the_board_device_and_list_inputs_names_it() {
    let mut s = open_ring();
    let inputs = s.list_inputs();
    assert!(
        inputs
            .iter()
            .any(|(owner, ch)| owner == "touch" && ch.key == "pressed"),
        "{inputs:?}"
    );
    // Two buttons expose `pressed`: a bare channel name is a typed ambiguity.
    assert!(matches!(
        s.set_input("pressed", 1.0),
        Err(SessionError::Input(SimInputError::Ambiguous {
            matches: 2,
            ..
        }))
    ));

    let mut s = open_ring_one_button();
    s.watch_logic(&[("gpio1", 13)]).unwrap();
    // Atomic: one bad set rejects the whole transaction and moves nothing.
    assert!(s
        .set_inputs(&[("pressed".into(), 1.0), ("no_such_channel".into(), 1.0)])
        .is_err());
    s.run_cycles(1_000).unwrap();
    assert!(
        s.logic(0).edges.is_empty(),
        "a rejected transaction drove the pin"
    );

    s.set_inputs(&[("pressed".into(), 1.0)]).unwrap();
    s.run_cycles(1_000).unwrap();
    s.set_input("pressed", 0.0).unwrap();
    s.run_cycles(1_000).unwrap();
    let values: Vec<bool> = s.logic(0).edges.iter().map(|e| e.value).collect();
    assert_eq!(values, vec![true, false], "press then release on gpio1.13");
}

#[test]
fn inspect_decodes_peripherals_and_the_attached_devices() {
    let mut s = open_ring();
    s.run_cycles(4_000).unwrap();
    let one = s.inspect(Some("twi21"));
    assert_eq!(one.peripherals.len(), 1);
    assert_eq!(one.peripherals[0].name, "twi21");
    let all = s.inspect(None);
    assert!(all.peripherals.iter().any(|p| p.name == "gpio1"));
    assert!(
        all.devices
            .iter()
            .any(|d| d.id == "touch" && d.attachment.transport == "gpio"),
        "the board-IO button is an attached device"
    );
    assert!(
        !one.peripherals[0].registers.is_empty(),
        "twi21 decodes its registers"
    );

    // Inspect is observation only: a run that inspects everything between
    // steps is the same run as one that never looks.
    let mut quiet = open_ring();
    let mut nosy = open_ring();
    for _ in 0..8 {
        quiet.run_cycles(500).unwrap();
        nosy.run_cycles(500).unwrap();
        let _ = nosy.inspect(None);
    }
    assert_eq!(quiet.frames(), nosy.frames());
    assert_eq!(quiet.uart_transcript(), nosy.uart_transcript());
}

#[test]
fn snapshot_restore_replays_identically() {
    let Some((mut s, _)) = open_arm() else {
        return;
    };
    s.run_for(Duration::from_millis(5)).unwrap();
    let snap = s.snapshot();
    let c = s.cycles();
    s.run_for(Duration::from_millis(5)).unwrap();
    let uart_after = s.uart_transcript();
    s.restore(&snap).unwrap();
    assert_eq!(s.cycles(), c);
    s.run_for(Duration::from_millis(5)).unwrap();
    assert_eq!(s.uart_transcript(), uart_after);
}

/// The CPU is the easy half of a snapshot. This one is taken mid-probe, with a
/// stimulus already applied, so a restore that brought back only registers
/// would replay different I²C transactions, a different console, or a
/// released button.
#[test]
fn restore_rewinds_device_bus_and_stimulus_state_too() {
    let mut s = open_ring();
    s.watch_logic(&[("gpio1", 13)]).unwrap();
    s.run_cycles(1_000).unwrap();
    s.set_pin("touch", true).unwrap();
    s.run_cycles(1_000).unwrap();
    let before = s.frames();
    let _ = s.read_uart();

    let snap = s.snapshot();
    let c = s.cycles();
    assert_eq!(snap.cycles(), c);

    // The same steps on both sides of the restore.
    let branch = |s: &mut Session| {
        assert_eq!(
            s.watch_logic(&[("gpio1", 13)]).unwrap(),
            vec![Some(true)],
            "the press applied before the snapshot is held"
        );
        s.set_pin("touch", false).unwrap();
        s.run_for(Duration::from_millis(20)).unwrap();
    };

    branch(&mut s);
    let frames_after = s.frames();
    let uart_after = s.read_uart();
    assert!(!frames_after.is_empty() && !uart_after.is_empty());

    s.restore(&snap).unwrap();
    assert_eq!(s.cycles(), c);
    assert!(s.read_uart().is_empty(), "the read cursor is restored too");
    assert!(s.frames().is_empty(), "so is the frame cursor");

    branch(&mut s);
    let key = |fs: &[labwired_core::session::Frame]| {
        fs.iter()
            .map(|f| (f.seq, f.cycle, f.bus.clone(), f.payload.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(key(&s.frames()), key(&frames_after));
    assert_eq!(s.read_uart(), uart_after);
    assert!(!before.is_empty());

    // A snapshot names the session it came from.
    let mut other = open_ring();
    assert!(matches!(other.restore(&snap), Err(SessionError::Other(_))));
}

/// Every symbol the smart-ring ELF defines resolves the same through the
/// session as through the loader.
#[test]
fn symbol_lookup_agrees_with_the_loader_on_every_name() {
    let f = common::smart_ring_fixture();
    let s = open_fixture(&f);
    let elf = goblin::elf::Elf::parse(&f.fw).unwrap();
    let mut checked = 0;
    for sym in elf.syms.iter() {
        let Some(name) = elf.strtab.get_at(sym.st_name).filter(|n| !n.is_empty()) else {
            continue;
        };
        assert_eq!(
            s.symbol(name).map(|v| v as u32),
            labwired_loader::resolve_symbol_in_elf(&f.fw, name),
            "{name}"
        );
        checked += 1;
    }
    assert!(
        checked > 20,
        "the probe ELF carries a symbol table ({checked})"
    );
}
