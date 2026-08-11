//! Two ESP32-C3 nodes running ONE image must start the owner's BLE Pong game.
//!
//! This is the end-to-end proof for connectionless BLE between two instances in
//! one lab. The sketch is the owner's published lab (api.labwired.com project
//! `c477f82961e86f601e7b908ae7e12311`, "BLE Pong — two ESP32-C3s") rather than
//! a purpose-built one, because the whole point is that a real user's real
//! sketch works.
//!
//! IT IS A FROZEN COPY, NOT A LIVE FETCH. CI has no network, so this test runs
//! a COMMITTED flash image (`fixtures/esp32c3-ble-pong-flash.bin`) built from a
//! COMMITTED copy of the sketch (`fixtures/esp32c3-ble-pong.ino`). Nothing here
//! contacts api.labwired.com, and nothing should: a published lab is a living
//! product artifact and is SUPPOSED to change, so this test must not depend on
//! what the owner has on the live site today.
//!
//! `tests/fixture_ble_pong_provenance.rs` pins the sha256 of BOTH files, so
//! editing the `.ino` without rebuilding the `.bin` — or swapping the `.bin`
//! without recording which source it came from — fails there instead of
//! silently testing firmware nobody runs. It is not `cfg`-gated, so unlike
//! this file it also runs on PRs. That is exactly how this file rotted once
//! already: the fixture was frozen at the #828 build while the published
//! sketch moved on repeatedly, and the test stayed green against firmware
//! that no longer existed — while the lab was visibly broken in the browser.
//!
//! The sketch elects its host over the air: each node publishes its state in
//! the manufacturer data of its advertisement, and in its scan callback
//!
//! ```c
//! if ((uint8_t)m[2] == myTag) return;   // our own frame
//! ...
//! if (!roleLocked) { if (!strapHost) isHost = (myTag < peerTag); roleLocked = true; }
//! ```
//!
//! where `myTag = BLEDevice::getAddress().getNative()[5]`. Two properties have
//! to hold for that to settle, and this file asserts each on its own so a
//! failure names which one broke:
//!
//! 1. **Distinct identity.** Two C3 dice in one lab must not share a Bluetooth
//!    device address. With a zero factory eFuse MAC on both, both nodes derive
//!    `tag=2`, every peer report looks like an echo of their own advertisement,
//!    and both stay GUEST forever with the ball frozen at spawn.
//! 2. **Delivery.** An advertising report transmitted by one controller has to
//!    reach the other's scan.
//! 3. **Both panels show the same game.** The guest's ball has to track the
//!    host's. This is the one that caught a real regression, and it is why the
//!    last assertion compares the two nodes rather than checking each alone.
//!
//! All three are observed the way a user observes them: from the serial the
//! sketch prints. Nothing here reads engine internals.
//!
//! # What property 3 caught (2026-08-07)
//!
//! A published revision set `PUBLISH_MS` to 700 while the game loop ran at
//! 50 Hz. The host's world snapshot then went out 1.4 times a second, and the
//! host's FIRST advertisement is emitted at the end of `setup()` while `isHost`
//! is still false — a 5-byte guest-shaped frame with no world state at all. So
//! for the whole first 700 ms the guest had nothing to draw: `haveHostFrame`
//! stayed false and it painted the static fallback. That is the "dead panel"
//! users saw. Given longer, the guest did receive frames — and repainted ONE
//! stale snapshot ~15 frames running, drifting up to 70 px from the host.
//!
//! The fix was not a faster radio: the host now advertises its VELOCITY and the
//! guest dead-reckons between packets, so motion is smooth at the loop rate
//! regardless of cadence. Cadence still matters for a different reason — see
//! the note on `PONG_CYCLES` about what this test's budget can and cannot see.
//!
//! The image is the flash the hosted PlatformIO toolchain builds from
//! `fixtures/esp32c3-ble-pong.ino`. Reproduce with `labwired_compile`, board
//! `esp32-c3-supermini`, language `arduino`, lib_deps
//! `adafruit/Adafruit SSD1306` + `adafruit/Adafruit GFX Library`, then
//! concatenate the returned flash images at their offsets (pad the gaps with
//! `0xFF`).

// RELEASE-ONLY. Two C3 mask-ROM boots plus enough advertising rounds for the
// election to settle is ~180M cycles; that is seconds in release and tens of
// minutes in debug, and every ordinary cargo step in `core-ci.yml` builds
// debug. The `Release-gated tests` step names this target explicitly and
// `release_only_cfg_ratchet` fails if it ever stops doing so — a release-only
// block that no lane compiles is worse than no test at all.
#![cfg(all(feature = "event-scheduler", not(debug_assertions)))]

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::boot::esp32c3_rom::{
    build_rom_boot_machine, c3_rom_data_init_writes, inject_rom_regions, RomBootOpts,
};
use labwired_core::boot::esp32s3_rom::RomImages;
use labwired_core::bus::SystemBus;
use labwired_core::cpu::RiscV;
use labwired_core::memory::ProgramImage;
use labwired_core::network::SimMqttFabric;
use labwired_core::peripherals::ble_air::BleAirBus;
use labwired_core::peripherals::nrf52::radio::VirtualAirBus;
use labwired_core::peripherals::rf_medium::{PathLossParams, RfMedium};
use labwired_core::{Arch, Bus, Cpu, Machine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

const ESP_IMAGE_HEADER_LEN: usize = 24;
const ESP_IMAGE_MAGIC: u8 = 0xE9;

fn esp32c3_bootloader_image(flash: &[u8]) -> ProgramImage {
    assert!(flash.len() > ESP_IMAGE_HEADER_LEN, "flash image truncated");
    assert_eq!(flash[0], ESP_IMAGE_MAGIC, "bad bootloader image magic");
    let segment_count = flash[1] as usize;
    let entry = u32::from_le_bytes(flash[4..8].try_into().unwrap()) as u64;
    let mut program = ProgramImage::new(entry, Arch::RiscV);
    let mut cursor = ESP_IMAGE_HEADER_LEN;
    for _ in 0..segment_count {
        let load_addr = u32::from_le_bytes(flash[cursor..cursor + 4].try_into().unwrap()) as u64;
        let len = u32::from_le_bytes(flash[cursor + 4..cursor + 8].try_into().unwrap()) as usize;
        cursor += 8;
        program.add_segment(load_addr, flash[cursor..cursor + len].to_vec());
        cursor += len;
    }
    program
}

struct Node {
    machine: Machine<RiscV>,
    serial: Arc<Mutex<Vec<u8>>>,
}

impl Node {
    fn console(&self) -> String {
        let bytes = self
            .serial
            .lock()
            .expect("serial sink not poisoned")
            .clone();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// One lab's radio fabric — the same trio `World` mints per lab
/// (`world.rs`), bound to both nodes of a pair via `attach_lab_air`.
///
/// Why this exists (issue #928): an `Esp32c3Bt` that nobody binds falls back
/// to the PROCESS-GLOBAL `default_ble_air_bus()`. This binary holds TWO
/// independent two-node labs — the election pair (`run_pong`) and the ADC
/// pair (`shipped_pong_firmware_observes_gp2y_…`) — and the test harness runs
/// them on parallel threads, so all four controllers advertised into one
/// shared air and the election lab heard the ADC lab's frames. Which frames
/// landed depended on thread interleaving, which is why the election flaked
/// ~2 runs in 3 while every solo run passed. The browser never hits this: a
/// lab gets its own worker there, and `World` mints one `BleAirBus` per lab —
/// binding the same way here restores that isolation.
struct LabAir {
    nrf: VirtualAirBus,
    ble: BleAirBus,
    fabric: SimMqttFabric,
}

impl LabAir {
    fn new() -> Self {
        let nrf = VirtualAirBus::new();
        nrf.attach_medium(RfMedium::new(1).with_params(PathLossParams::default()));
        Self {
            nrf,
            ble: BleAirBus::new(),
            fabric: SimMqttFabric::new(),
        }
    }
}

/// The browser fast-start assembly for one C3 node, identical to the shipped
/// lab batch gate's `build_lab` — same ROM injection, same rom-boot options,
/// same browser tick policy — so what runs here is what runs in the tab.
fn build_node(flash: &[u8], lab: &LabAir, node_id: &str) -> Node {
    let chip = ChipDescriptor::from_file(root().join("../../configs/chips/esp32c3.yaml"))
        .expect("load esp32c3 chip yaml");
    let manifest =
        SystemManifest::from_file(root().join("../../configs/systems/esp32c3-ble-pong.yaml"))
            .expect("load ble-pong system yaml");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build ble-pong bus");

    let irom = std::fs::read(root().join("roms/esp32c3/esp32c3_rom.bin")).expect("read C3 IROM");
    let drom = std::fs::read(root().join("roms/esp32c3/esp32c3_drom.bin")).expect("read C3 DROM");
    assert!(
        inject_rom_regions(
            &mut bus,
            &RomImages {
                irom: irom.clone(),
                drom,
            },
        ),
        "chip yaml must declare the C3 IROM region"
    );
    for (dst, bytes) in c3_rom_data_init_writes(&irom) {
        for (i, b) in bytes.iter().enumerate() {
            let _ = bus.write_u8(dst as u64 + i as u64, *b);
        }
    }

    let serial = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(serial.clone(), false);

    let bootloader = esp32c3_bootloader_image(flash);
    let mut machine = build_rom_boot_machine(
        bus,
        flash.to_vec(),
        RomBootOpts {
            // Unpinned: each node is its own die, exactly as the browser builds
            // one bridge per MCU on the canvas.
            pinned_efuse_mac: None,
            usb_serial_sink: None,
        },
        |c| c,
    );
    // Bind this node's radios to the lab-private air (see LabAir). Both nodes
    // of a pair get clones of the same trio, so they hear each other and no
    // other lab in the process.
    machine.bus.attach_lab_air(
        node_id,
        lab.nrf.clone(),
        lab.ble.clone(),
        lab.fabric.clone(),
    );
    for segment in &bootloader.segments {
        if machine.bus.flash.load_from_segment(segment)
            || machine.bus.ram.load_from_segment(segment)
            || machine
                .bus
                .extra_mem
                .iter_mut()
                .any(|m| m.load_from_segment(segment))
        {
            continue;
        }
        for (i, byte) in segment.data.iter().enumerate() {
            machine
                .bus
                .write_u8(segment.start_addr + i as u64, *byte)
                .expect("load bootloader segment");
        }
    }
    let sp_top = (chip.ram.base + labwired_config::parse_size(&chip.ram.size).unwrap_or(0)) as u32;
    machine.cpu.set_sp(sp_top & !0xF);
    machine.cpu.set_pc(bootloader.entry_point as u32);

    let rec = machine.bus.max_safe_tick_interval();
    machine.config.peripheral_tick_interval = rec;
    machine.bus.config.peripheral_tick_interval = rec;
    machine.config.idle_fast_forward_enabled = true;
    Node { machine, serial }
}

/// Slice both nodes in lockstep. Advertising is latest-value-wins with a
/// bounded backlog, so a node that runs a huge uninterrupted batch would talk
/// past its peer's scan window — the same reasoning the browser's per-chip
/// ticker uses for wire-linked chips.
const SLICE_CYCLES: usize = 100_000;

fn run_pong(cycles_per_node: usize) -> (Node, Node) {
    let flash = std::fs::read(root().join("tests/fixtures/esp32c3-ble-pong-flash.bin"))
        .expect("read BLE Pong flash image");
    let lab = LabAir::new();
    let mut a = build_node(&flash, &lab, "nodeA");
    let mut b = build_node(&flash, &lab, "nodeB");
    let mut done = 0usize;
    while done < cycles_per_node {
        let slice = SLICE_CYCLES.min(cycles_per_node - done);
        for node in [&mut a, &mut b] {
            for _ in 0..slice {
                if node.machine.step().is_err() {
                    break;
                }
            }
        }
        done += slice;
    }
    (a, b)
}

/// The `tag=` a node printed in its `ROLE …` banner — the last byte of the BLE
/// device address the stack handed the sketch.
fn ble_tag(console: &str) -> u8 {
    let line = console
        .lines()
        .find(|l| l.starts_with("ROLE "))
        .unwrap_or_else(|| panic!("no ROLE banner in console:\n{console}"));
    let tag = line
        .split("tag=")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or_else(|| panic!("no tag= in {line:?}"));
    tag.parse().unwrap_or_else(|e| panic!("tag {tag:?}: {e}"))
}

/// The last per-loop status line, e.g. `H ball=64,32 me=25 peer=24 score=0:0 rally=0`.
fn last_status(console: &str) -> String {
    console
        .lines()
        .rev()
        .find(|l| l.contains(" ball=") && l.contains(" rally="))
        .unwrap_or_else(|| panic!("no status line in console:\n{console}"))
        .to_string()
}

/// The last `n` non-empty serial lines, oldest first.
fn tail(console: &str, n: usize) -> String {
    let lines: Vec<&str> = console.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

fn ball(status: &str) -> (i32, i32) {
    let f = status
        .split(" ball=")
        .nth(1)
        .and_then(|r| r.split_whitespace().next())
        .unwrap_or_else(|| panic!("no ball= in {status:?}"));
    let (x, y) = f.split_once(',').expect("ball=x,y");
    (x.parse().expect("ball x"), y.parse().expect("ball y"))
}

fn paddle(status: &str) -> i32 {
    status
        .split(" me=")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or_else(|| panic!("no me= in {status:?}"))
        .parse()
        .unwrap_or_else(|e| panic!("bad me= in {status:?}: {e}"))
}

/// Cycles per node. The C3 rom-boot path reaches `setup()`'s ROLE banner in
/// roughly 45M and needs a few advertising rounds after that for the election
/// to settle and the ball to move.
///
/// ⚠ KNOW WHAT THIS BUDGET CANNOT SEE. 90M cycles at 160 MHz is ~560 ms of
/// simulated time, and ~45M of it is ROM boot — so the sketch's `loop()` only
/// gets ~140 ms, about seven iterations at `delay(20)`. Any firmware behaviour
/// whose period is longer than that is INVISIBLE here, and will read as a pass
/// or as a spurious failure depending on which side of the assertion it falls.
/// That is not hypothetical: with `PUBLISH_MS` at 700 the host's second
/// advertisement simply never happened inside the window, so what the test
/// reported was "no host frame ever arrived" — true, but it could not
/// distinguish "the cadence is too slow" from "delivery is broken".
///
/// Raising it is expensive (the run is the whole cost of this test), so the
/// budget stays where the gate needs it and the long window lives in its own
/// lane instead: `esp32c3-ble-pong-soak` in `.github/workflows/core-nightly.yml`
/// runs this same file with `PONG_SOAK_CYCLES=480000000`, ~30 republish
/// periods. No gate that has to be fast ever sets the variable, so the constant
/// below is still what gates a merge — do not raise it to buy coverage the
/// nightly already has.
const PONG_CYCLES: usize = 90_000_000;

/// ONE two-node run, shared by both tests. The run is the expensive part and
/// both properties are readable from the same serial, so paying for it twice
/// would double the release lane for nothing. Each property still gets its own
/// `#[test]` so a failure names which one broke.
fn consoles() -> &'static (String, String) {
    static RUN: std::sync::OnceLock<(String, String)> = std::sync::OnceLock::new();
    RUN.get_or_init(|| {
        // Soak override — see the note on PONG_CYCLES. Set ONLY by the nightly
        // `esp32c3-ble-pong-soak` job and by hand during characterisation; every
        // gate that has to be fast leaves it unset and runs the constant.
        // It exists because answering "does this advertising cadence stall the
        // twin's RW-BLE controller?" needs tens of republish periods, and
        // hand-editing the constant to find out is how the edited value gets
        // committed by accident.
        let budget = std::env::var("PONG_SOAK_CYCLES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(PONG_CYCLES);
        let (a, b) = run_pong(budget);
        (a.console(), b.console())
    })
}

/// Property 1: two dice, two addresses. A lab with two of the same board is the
/// ordinary case, and the sketch's self-filter (`m[2] == myTag`) is the
/// ordinary way to write connectionless BLE — a shared address makes every peer
/// advertisement indistinguishable from an echo.
#[test]
fn two_c3_nodes_in_one_lab_do_not_share_a_bluetooth_address() {
    let (ca, cb) = consoles();
    let (ta, tb) = (ble_tag(ca), ble_tag(cb));
    assert_ne!(
        ta, tb,
        "both nodes report BLE address byte tag={ta}; a peer's advertisement is \
         then indistinguishable from the node's own and the game never starts"
    );
}

/// Property 2: the game starts. Exactly one node takes the host role, and the
/// ball leaves its spawn — which can only happen once host election has run,
/// i.e. once a peer advertising report was delivered and accepted.
#[test]
fn two_c3_nodes_running_one_ble_pong_image_start_the_game() {
    let (ca, cb) = consoles();
    let (sa, sb) = (last_status(ca), last_status(cb));
    // The evidence a human reads out of a CI log, printed on pass as well as
    // fail: what each node decided it was, and what it was painting.
    eprintln!("nodeA tag={} {sa}", ble_tag(ca));
    eprintln!("nodeB tag={} {sb}", ble_tag(cb));
    // On failure the LAST line is rarely enough — "the guest is repainting one
    // stale snapshot" and "the guest never received anything" have identical
    // final lines and completely different causes. The tail distinguishes them
    // without a second run, which on this test costs two mask-ROM boots.
    eprintln!("--- nodeA tail ---\n{}", tail(ca, 8));
    eprintln!("--- nodeB tail ---\n{}", tail(cb, 8));

    let hosts = [&sa, &sb].iter().filter(|s| s.starts_with("H ")).count();
    assert_eq!(
        hosts, 1,
        "exactly one node must elect HOST\n  nodeA: {sa}\n  nodeB: {sb}"
    );

    // The host owns ball physics; the guest paints the host's snapshot. Neither
    // may still be sitting on the spawn point.
    for (name, status) in [("nodeA", &sa), ("nodeB", &sb)] {
        assert_ne!(
            ball(status),
            (64, 32),
            "{name} ball never left spawn — no host frame ever arrived: {status}"
        );
    }
    // Same picture on both panels: the guest mirrors the host's ball.
    assert!(
        ball(&sa).0.abs_diff(ball(&sb).0) <= 8,
        "the two nodes are painting different worlds\n  nodeA: {sa}\n  nodeB: {sb}"
    );
}

#[test]
fn shipped_pong_firmware_observes_gp2y_distance_through_gpio1_adc() {
    let flash = std::fs::read(root().join("tests/fixtures/esp32c3-ble-pong-flash.bin"))
        .expect("read BLE Pong flash image");
    // A second, independent lab — its own air, so its advertisements never
    // reach the election pair above even when both run concurrently (#928).
    let lab = LabAir::new();
    let mut near = build_node(&flash, &lab, "near");
    let mut far = build_node(&flash, &lab, "far");
    near.machine
        .set_input_on("vl", "distance", 100.0)
        .expect("drive near GP2Y distance");
    far.machine
        .set_input_on("vl", "distance", 800.0)
        .expect("drive far GP2Y distance");

    for _ in 0..PONG_CYCLES {
        near.machine.step().expect("near node steps");
        far.machine.step().expect("far node steps");
    }

    let near_status = last_status(&near.console());
    let far_status = last_status(&far.console());
    eprintln!("near: {near_status}");
    eprintln!("far:  {far_status}");
    assert!(
        paddle(&near_status) < paddle(&far_status),
        "near and far stimuli must move the firmware paddle through GPIO1 ADC\n\
         near: {near_status}\nfar:  {far_status}",
    );
}
