// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// Offline capture harness: runs the real nokia5110-invaders-lab firmware,
// scripts the host-controlled HC-SR04 "hand distance" so the ship sweeps
// back and forth, samples the PCD8544 framebuffer on a fixed cycle cadence,
// and writes a packed `.bin` consumed by the landing-page hero renderer.
//
// Run (slow, offline — gated behind --ignored):
//   cargo test -p labwired-core --test capture_nokia5110_frames -- --ignored --nocapture

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::cortex_m::CortexM;
use labwired_core::peripherals::components::Pcd8544;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::Machine;
use std::path::PathBuf;
use std::process::Command;

type Cm = Machine<CortexM>;

const W: usize = 84;
const H: usize = 48;
const BANKS: usize = H / 8; // 6
const FRAME_BYTES: usize = W * BANKS; // 504

const FPS: u8 = 15;
const FRAME_COUNT: usize = 90; // ~6 s at 15 fps
const CPU_HZ: u64 = 4_000_000; // 4 MHz MSI, per the example
const CYCLES_PER_FRAME: u64 = CPU_HZ / FPS as u64; // ~266_666
const WARMUP_CYCLES: u64 = 9_000_000; // step past the splash + 8M-cycle hold

const NEAR_CM: f32 = 8.0;
const FAR_CM: f32 = 250.0;

fn ensure_firmware_built() -> PathBuf {
    let elf = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/thumbv7em-none-eabihf/release/nokia5110-invaders-lab");
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/nokia5110-invaders-lab/src/main.rs");
    if let (Ok(e), Ok(s)) = (std::fs::metadata(&elf), std::fs::metadata(&src)) {
        if e.modified().unwrap() >= s.modified().unwrap() {
            return elf;
        }
    }
    let status = Command::new("cargo")
        .args([
            "build",
            "-p",
            "nokia5110-invaders-lab",
            "--target",
            "thumbv7em-none-eabihf",
            "--release",
        ])
        .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .status()
        .expect("cargo build nokia5110-invaders-lab");
    assert!(status.success(), "nokia5110-invaders-lab build failed");
    assert!(elf.exists(), "ELF not found at {elf:?}");
    elf
}

fn build_machine(elf: &PathBuf) -> Cm {
    let system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/nokia5110-invaders-lab/system.yaml");
    let manifest = SystemManifest::from_file(&system_path).expect("load manifest");
    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let image = labwired_loader::load_elf(elf).expect("parse ELF");
    machine.load_firmware(&image).expect("load firmware");
    machine
}

fn framebuffer(machine: &Cm) -> Vec<u8> {
    let idx = machine.bus.find_peripheral_index_by_name("spi1").unwrap();
    let spi = machine.bus.peripherals[idx]
        .dev
        .as_any()
        .unwrap()
        .downcast_ref::<labwired_core::peripherals::spi::Spi>()
        .unwrap();
    spi.attached_devices
        .iter()
        .find_map(|d| d.as_any().and_then(|a| a.downcast_ref::<Pcd8544>()))
        .expect("PCD8544 attached to spi1")
        .framebuffer()
        .to_vec()
}

/// Left-most column of the player ship (bank 5 holds the ship rows).
fn ship_left(fb: &[u8]) -> Option<usize> {
    (0..W).find(|&x| fb[5 * W + x] != 0)
}

fn step_frames(machine: &mut Cm, steps: u64) {
    for _ in 0..steps {
        machine.step().expect("step");
    }
}

/// Triangle sweep of the "hand distance" across the capture window: two full
/// near->far->near passes, so the ship visibly tracks back and forth.
fn scripted_distance_cm(frame: usize, total: usize) -> f32 {
    let phase = (frame as f32) / (total as f32) * 2.0; // 0..2 (two passes)
    let tri = 1.0 - (2.0 * (phase - phase.floor()) - 1.0).abs(); // 0..1..0
    NEAR_CM + (FAR_CM - NEAR_CM) * tri
}

/// Collect FRAME_COUNT framebuffers while scripting the ship.
fn collect_frames() -> Vec<Vec<u8>> {
    let elf = ensure_firmware_built();
    let mut machine = build_machine(&elf);
    machine.bus.hcsr04[0].set_distance_cm(NEAR_CM);
    step_frames(&mut machine, WARMUP_CYCLES);

    let mut frames = Vec::with_capacity(FRAME_COUNT);
    for i in 0..FRAME_COUNT {
        machine.bus.hcsr04[0].set_distance_cm(scripted_distance_cm(i, FRAME_COUNT));
        step_frames(&mut machine, CYCLES_PER_FRAME);
        let fb = framebuffer(&machine);
        assert_eq!(fb.len(), FRAME_BYTES, "framebuffer must be 504 bytes");
        frames.push(fb);
    }
    frames
}

#[test]
#[ignore = "slow: builds + runs the Space Invaders firmware in-sim (~minutes)"]
fn collects_animated_frames() {
    let frames = collect_frames();
    assert_eq!(frames.len(), FRAME_COUNT);
    assert!(
        frames.iter().all(|f| f.iter().any(|&b| b != 0)),
        "every captured frame should have non-blank pixels"
    );
    let positions: Vec<Option<usize>> = frames.iter().map(|f| ship_left(f)).collect();
    let distinct: std::collections::HashSet<_> = positions.iter().flatten().collect();
    assert!(
        distinct.len() >= 3,
        "ship should occupy several distinct x positions, got {distinct:?}"
    );
}

fn pack(frames: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + frames.len() * FRAME_BYTES);
    out.extend_from_slice(b"NK51"); // magic
    out.push(1); // version
    out.push(FPS); // fps
    out.extend_from_slice(&(frames.len() as u16).to_le_bytes()); // frame count
    out.push(W as u8); // width = 84
    out.push(H as u8); // height = 48
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved -> 12-byte header
    for f in frames {
        out.extend_from_slice(f);
    }
    out
}

#[test]
fn pack_layout_is_well_formed() {
    // Two synthetic 504-byte frames; checks header offsets without running the sim.
    let frames = vec![vec![0u8; FRAME_BYTES], vec![0u8; FRAME_BYTES]];
    let bin = pack(&frames);
    assert_eq!(&bin[0..4], b"NK51");
    assert_eq!(bin[4], 1); // version
    assert_eq!(bin[5], FPS);
    assert_eq!(u16::from_le_bytes([bin[6], bin[7]]) as usize, 2);
    assert_eq!(bin[8], W as u8);
    assert_eq!(bin[9], H as u8);
    assert_eq!(bin.len(), 12 + 2 * FRAME_BYTES);
}

#[test]
#[ignore = "slow: writes invaders-frames.bin from a real sim run"]
fn write_frames_bin() {
    let frames = collect_frames();
    let bin = pack(&frames);
    let out = std::env::var("INVADERS_FRAMES_OUT").unwrap_or_else(|_| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/invaders-frames.bin")
            .to_string_lossy()
            .into_owned()
    });
    std::fs::write(&out, &bin).expect("write invaders-frames.bin");
    println!(
        "wrote {} bytes ({} frames) to {out}",
        bin.len(),
        frames.len()
    );
}
