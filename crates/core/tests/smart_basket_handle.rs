// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SmartBasket handle end to end: builds `examples/smart-basket-handle` and
//! runs the REAL firmware on the ESP32-S3 with the board's four chip models
//! attached from the example's own `system.yaml` (OV3660 on I2C1/DVP, ADXL345
//! and INA219 on I2C0, HX711 bit-bang, DOCK input).
//!
//! What the tests drive and assert:
//!   * Weight stimulus on the HX711 moves the session: each settled weight
//!     change ≥ 40 g yields one `EVT` with a two-frame JPEG burst whose bytes
//!     land in the session store with a CRC-32.
//!   * Camera stimulus (brightness) reaches the frames: the dim frame decodes
//!     darker than the bright one after an actual JPEG decode of the dumped
//!     bytes.
//!   * DOCK rising dumps the session over USB-Serial-JTAG; every dumped frame
//!     decodes with `jpeg_decoder`, its length and CRC match the header, and
//!     the per-event metadata (weight, delta) matches the stimulus.
//!   * Battery thresholds: INA219 bus voltage below 3.30 V / 3.20 V logs
//!     LOW_BATT / CRIT_BATT, a critical pack refuses new captures
//!     (`CRIT_SKIP`), and recovery re-enables them.
//!   * IMU motion without a weight change is not a capture (`ACCEL`, no EVT).
//!
//! These run the production attach path (`attach_esp32_external_devices`)
//! against the example's real manifest — not a test-only wiring.

#![cfg(feature = "esp32s3-fixtures")]

mod common;
use labwired_core::boot::esp32s3::{fast_boot, BootOpts};
use labwired_core::bus::SystemBus;
use labwired_core::system::xtensa::{
    attach_esp32_external_devices, configure_xtensa_esp32s3, Esp32s3Opts,
};
use labwired_core::{AdvanceRequest, AdvanceStop, Cpu, Machine};
use std::sync::{Arc, Mutex};

/// DOCK contact pin (system.yaml: DOCK input GPIO48).
const DOCK_PIN: u8 = 48;

/// Chunk size for `Machine::advance`, mirroring the CLI's Xtensa batched loop.
const CHUNK: u64 = 2_000_000;

struct Rig {
    machine: Machine<labwired_core::cpu::XtensaLx7>,
    jtag: Arc<Mutex<Vec<u8>>>,
    instructions: u64,
    max_instructions: u64,
}

impl Rig {
    /// Build the S3 bus from the example's real manifest and fast-boot the
    /// built firmware.
    fn new(max_instructions: u64) -> Rig {
        let elf_path = common::ensure_esp_firmware_built(
            "smart-basket-handle",
            "target/xtensa-esp32s3-none-elf/release/smart-basket-handle",
        );
        let elf_bytes = std::fs::read(&elf_path).expect("read firmware ELF");

        let mut bus = SystemBus::new();
        // 80 MHz keeps the step budgets affordable: guest time is
        // (steps / cpu_hz), and every pipeline deadline here is in
        // milliseconds, so the lower clock costs nothing in coverage.
        let opts = Esp32s3Opts {
            cpu_clock_hz: 20_000_000,
            ..Esp32s3Opts::default()
        };
        let wiring = configure_xtensa_esp32s3(&mut bus, &opts);

        let yaml = common::root("examples/smart-basket-handle/system.yaml");
        let manifest =
            labwired_config::SystemManifest::from_file(&yaml).expect("parse example system.yaml");
        attach_esp32_external_devices(&mut bus, &manifest)
            .expect("attach the handle board from its own manifest");
        bus.refresh_peripheral_index();

        let jtag = Arc::new(Mutex::new(Vec::<u8>::new()));
        assert!(
            bus.attach_usb_serial_jtag_sink(jtag.clone()),
            "the S3 bus must expose USB-Serial-JTAG for the device console"
        );

        let icache_backing = wiring.icache_backing.clone();
        let dcache_backing = wiring.dcache_backing.clone();
        let mut cpu = wiring.cpu;
        fast_boot(
            &elf_bytes,
            &mut bus,
            &mut cpu,
            &BootOpts {
                stack_top_fallback: 0x3FCD_FFF0,
                icache_backing: Some(icache_backing),
                dcache_backing: Some(dcache_backing),
                factory_flash_base: None,
            },
        )
        .expect("fast_boot");

        // Same batched hot path as `labwired run` on Xtensa.
        let mut machine = Machine::new(cpu, bus);
        let interval = machine.bus.max_safe_tick_interval();
        machine.config.peripheral_tick_interval = interval;
        machine.bus.config.peripheral_tick_interval = interval;

        Rig {
            machine,
            jtag,
            instructions: 0,
            max_instructions,
        }
    }

    fn console(&self) -> String {
        String::from_utf8_lossy(&self.jtag.lock().unwrap()).to_string()
    }

    /// Advance one batch and return whether the machine still makes progress.
    fn advance_batch(&mut self, fuel: u64) -> bool {
        let before = self.machine.step_profile().cpu_instructions;
        match self.machine.advance(AdvanceRequest::run(Some(fuel))) {
            Ok(report) => {
                let delta = self.machine.step_profile().cpu_instructions - before;
                self.instructions += delta;
                match report.stop {
                    AdvanceStop::NoProgress => false,
                    _ => delta > 0,
                }
            }
            Err(e) => panic!(
                "simulator error at pc=0x{:08x}: {e}\nconsole:\n{}",
                self.machine.cpu.get_pc(),
                self.console()
            ),
        }
    }

    /// Run batches until `pred(&console)` holds. Panics with the console tail
    /// on exhaustion so a stuck pipeline is diagnosable, not just red.
    fn run_until(&mut self, what: &str, pred: impl Fn(&str) -> bool) {
        while self.instructions < self.max_instructions {
            if pred(&self.console()) {
                return;
            }
            let fuel = CHUNK.min(self.max_instructions - self.instructions);
            if !self.advance_batch(fuel) {
                break;
            }
        }
        if pred(&self.console()) {
            return;
        }
        let text = self.console();
        let tail: String = text
            .chars()
            .rev()
            .take(2000)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        panic!(
            "timed out waiting for {what} after {} instructions; console tail:\n{tail}",
            self.instructions
        );
    }

    /// Run a bounded amount of guest execution so triggers and polls that fire
    /// on their own schedule (battery 1 Hz, settle windows) get their chance.
    fn run_instructions(&mut self, n: u64) {
        let target = self.instructions + n;
        while self.instructions < target && self.instructions < self.max_instructions {
            let fuel = CHUNK.min(target - self.instructions);
            if !self.advance_batch(fuel) {
                break;
            }
        }
    }

    fn set_weight_g(&mut self, g: f64) {
        self.machine
            .bus
            .set_input(Some("scale"), "weight", g)
            .expect("HX711 accepts weight stimulus");
    }

    fn set_imu_g(&mut self, x: f64, z: f64) {
        self.machine
            .bus
            .set_input(Some("imu"), "x", x)
            .expect("imu x");
        self.machine
            .bus
            .set_input(Some("imu"), "z", z)
            .expect("imu z");
    }

    fn set_cam_brightness(&mut self, pct: f64) {
        self.machine
            .bus
            .set_input(Some("cam"), "brightness", pct)
            .expect("camera brightness stimulus");
    }

    fn set_batt_v(&mut self, v: f64) {
        self.machine
            .bus
            .set_input(Some("batt"), "bus_voltage", v)
            .expect("INA219 bus voltage stimulus");
    }

    fn set_dock(&mut self, high: bool) {
        let idx = self
            .machine
            .bus
            .find_peripheral_index_by_name("gpio")
            .expect("S3 gpio");
        assert!(
            self.machine.bus.peripherals[idx]
                .dev
                .set_gpio_input(DOCK_PIN, high),
            "gpio must accept DOCK input on pin {DOCK_PIN}"
        );
    }
}

/// Parse the console dump: `FRAME i=... len=... crc=0x...` header line,
/// `H <hex>` payload lines, then `ENDFRAME i=...`. Hex keeps the dump
/// re-syncable on newlines even though the JTAG printer can drop the tail of
/// a burst (so a raw binary stream is not trustworthy over this link).
/// Returns `(len, crc, bytes)` per frame in dump order.
fn dump_frames(text: &str) -> Vec<(usize, u32, Vec<u8>)> {
    let mut out = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let Some(rest) = line.strip_prefix("FRAME ") else {
            continue;
        };
        let field = |k: &str| -> String {
            rest.split(' ')
                .find_map(|f| f.strip_prefix(k))
                .unwrap_or_else(|| panic!("FRAME header missing {k}: {line}"))
                .to_string()
        };
        let len: usize = field("len=").parse().expect("frame len");
        let crc: u32 = u32::from_str_radix(&field("crc=0x"), 16).expect("frame crc");
        let mut bytes = Vec::with_capacity(len);
        loop {
            let Some(hexline) = lines.next() else {
                panic!("dump ended before ENDFRAME: {line}");
            };
            if let Some(hex) = hexline.strip_prefix("H ") {
                for pair in hex.as_bytes().chunks(2) {
                    if pair.len() != 2 {
                        panic!("odd hex line: {hexline}");
                    }
                    let hi = (pair[0] as char).to_digit(16).expect("hex digit") as u8;
                    let lo = (pair[1] as char).to_digit(16).expect("hex digit") as u8;
                    bytes.push((hi << 4) | lo);
                }
            } else if hexline.starts_with("ENDFRAME ") {
                break;
            } else {
                continue; // a dropped tail cannot fake a line; skip junk
            }
        }
        assert_eq!(bytes.len(), len, "hex payload length for FRAME {line}");
        out.push((len, crc, bytes));
    }
    out
}

/// Parse an `EVT`/`EVENT` line's numeric field, e.g. `dw=350` -> 350.
fn field_i64(line: &str, key: &str) -> i64 {
    line.split(' ')
        .find_map(|f| f.strip_prefix(key))
        .unwrap_or_else(|| panic!("no {key} in {line:?}"))
        .parse()
        .expect("numeric field")
}

/// CRC-32 (IEEE 802.3, reflected), the firmware's own bitwise algorithm.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Mean luma of a decoded RGB image — enough to tell a bright frame from a
/// dim one without depending on exact values.
fn mean_luma(px: &[u8]) -> u32 {
    let n = (px.len() / 3).max(1) as u64;
    let sum: u64 = px
        .chunks(3)
        .map(|p| (p[0] as u64) * 3 + (p[1] as u64) * 4 + p[2] as u64)
        .sum();
    (sum / n) as u32
}

#[test]
fn weight_events_battery_and_checkout_dump_end_to_end() {
    let mut rig = Rig::new(400_000_000);

    // Boot, camera probe, battery first read, tare all land before the loop.
    rig.run_until("boot+tare", |c| c.contains("TARE "));

    // ── Event 0: 350 g placed, IMU shaken, camera scene bright ─────────────
    rig.set_imu_g(0.7, 1.0); // |a| ≈ 1.22 g: clearly above the motion band
    rig.set_weight_g(350.0);
    rig.run_until("EVT 0", |c| c.contains("EVT i=0"));
    rig.set_imu_g(0.0, 1.0); // back to rest

    // ── Event 1: 900 g total (delta 550 g), camera scene dim ──────────────
    rig.set_cam_brightness(20.0);
    rig.set_weight_g(900.0);
    rig.run_until("EVT 1", |c| c.contains("EVT i=1"));

    // ── Event 2: weight removed to 200 g (delta −700 g), bright again ─────
    rig.set_cam_brightness(100.0);
    rig.set_weight_g(200.0);
    rig.run_until("EVT 2", |c| c.contains("EVT i=2"));

    // ── Battery: low, critical, capture refused, recovery ─────────────────
    rig.set_batt_v(3.25);
    rig.run_until("LOW_BATT", |c| c.contains("LOW_BATT"));
    rig.set_batt_v(3.10);
    rig.run_until("CRIT_BATT", |c| c.contains("CRIT_BATT"));

    rig.set_weight_g(1200.0); // critical pack: new session refused
    rig.run_until("CRIT_SKIP", |c| c.contains("CRIT_SKIP"));

    rig.set_batt_v(4.00); // charger attached: recover
    rig.run_until("BATT recovery", |c| c.contains("BATT mv=4000"));

    // ── Checkout: dock the handle; the session dumps ──────────────────────
    rig.set_dock(true);
    rig.run_until("DUMP end", |c| c.contains("DUMP end"));

    let text = rig.console();

    // The capture events carry the weight context the stimulus set.
    let ev_lines: Vec<&str> = text.lines().filter(|l| l.starts_with("EVT i=")).collect();
    assert_eq!(ev_lines.len(), 3, "three capture events; console:\n{text}");
    // The settling filter can stop a few grams short of the stimulus
    // (SETTLE_TOL_MG), so the assertions are tolerant bands around it.
    let (w0, dw0) = (field_i64(ev_lines[0], "w="), field_i64(ev_lines[0], "dw="));
    assert!(
        (300..=350).contains(&w0) && (300..=350).contains(&dw0),
        "event 0 weight context: {}",
        ev_lines[0]
    );
    let (w1, dw1) = (field_i64(ev_lines[1], "w="), field_i64(ev_lines[1], "dw="));
    assert!(
        (840..=900).contains(&w1) && (490..=600).contains(&dw1),
        "event 1 weight context: {}",
        ev_lines[1]
    );
    let (w2, dw2) = (field_i64(ev_lines[2], "w="), field_i64(ev_lines[2], "dw="));
    assert!(
        (180..=220).contains(&w2) && (-700..=-600).contains(&dw2),
        "event 2 weight context: {}",
        ev_lines[2]
    );
    assert!(
        field_i64(ev_lines[0], "a=") > 0,
        "event 0 recorded the IMU motion peak: {}",
        ev_lines[0]
    );
    // Event 2 happens long after the IMU was put back at rest and after
    // event 1's capture cleared the peak: it must be a quiet placement.
    assert_eq!(
        field_i64(ev_lines[2], "a="),
        0,
        "event 2 was a quiet placement: {}",
        ev_lines[2]
    );

    // A critical pack refuses, and never captures, the 1200 g placement.
    assert!(
        !text.contains("EVT i=3"),
        "no capture while the pack is critical"
    );

    // The dump has 3 events × 2 frames, with matching metadata.
    assert!(
        text.contains("DUMP events=3 frames=6"),
        "dump header:\n{text}"
    );
    let event_lines: Vec<&str> = text.lines().filter(|l| l.starts_with("EVENT ")).collect();
    assert_eq!(event_lines.len(), 3, "EVENT metadata rows");
    assert_eq!(field_i64(event_lines[0], "w="), w0, "{}", event_lines[0]);
    assert_eq!(field_i64(event_lines[0], "dw="), dw0, "{}", event_lines[0]);
    assert_eq!(field_i64(event_lines[1], "w="), w1, "{}", event_lines[1]);
    assert_eq!(field_i64(event_lines[2], "w="), w2, "{}", event_lines[2]);

    let frames = dump_frames(&text);
    assert_eq!(frames.len(), 6, "six dumped frames");

    // Every frame is a real JPEG: SOI marker, length/CRC as dumped, decodable.
    let mut decoded = Vec::new();
    for (i, (_len, crc, bytes)) in frames.iter().enumerate() {
        assert_eq!(&bytes[..3], &[0xFF, 0xD8, 0xFF], "frame {i} SOI");
        assert_eq!(crc32(bytes), *crc, "frame {i} CRC matches its header");
        let mut dec = jpeg_decoder::Decoder::new(&bytes[..]);
        let px = dec
            .decode()
            .unwrap_or_else(|e| panic!("frame {i} decodes: {e}"));
        let info = dec.info().expect("jpeg info");
        assert_eq!((info.width, info.height), (160, 120), "frame {i} window");
        decoded.push(mean_luma(&px));
    }

    // Camera stimulus reached the pixels: event 0's frames are bright, event
    // 1's (brightness 20 %) are clearly darker, event 2 (bright again) is not.
    assert!(
        decoded[0] > decoded[2] * 2,
        "dim scene must decode darker: bright={} dim={}",
        decoded[0],
        decoded[2]
    );
    assert!(
        decoded[4] > decoded[2] * 2,
        "third event is bright again: {} vs dim {}",
        decoded[4],
        decoded[2]
    );

    // Event metadata lengths match the frames the store actually held.
    let f0_len: usize = event_lines[0]
        .split(' ')
        .find_map(|f| f.strip_prefix("f0len="))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(f0_len, frames[0].0, "EVENT f0len ties to FRAME 0");

    // Undocking resets the session for the next shopper.
    rig.set_dock(false);
    rig.run_until("SESSION reset", |c| c.contains("SESSION reset"));

    // Sanity: no simulator faults anywhere in the run.
    assert!(!text.contains("panic"), "no firmware panic");
}

#[test]
fn imu_motion_without_weight_change_is_not_a_capture() {
    let mut rig = Rig::new(150_000_000);
    rig.run_until("boot+tare", |c| c.contains("TARE "));

    // Jostle the basket: motion, but no weight change.
    rig.set_imu_g(1.5, 1.0);
    rig.run_instructions(15_000_000); // ~0.75 s of guest time; polls + print rate
    rig.set_imu_g(0.0, 1.0);
    rig.run_instructions(8_000_000);

    let text = rig.console();
    assert!(
        text.contains("ACCEL "),
        "motion must be observed and logged: {text}"
    );
    assert!(
        text.contains("EVT_DROP") || !text.contains("EVT i="),
        "no capture event without a weight change: {text}"
    );

    // Docking dumps an empty session.
    rig.set_dock(true);
    rig.run_until("empty DUMP", |c| c.contains("DUMP end"));
    let text = rig.console();
    assert!(
        text.contains("DUMP events=0 frames=0"),
        "empty session dumps empty: {text}"
    );
}
