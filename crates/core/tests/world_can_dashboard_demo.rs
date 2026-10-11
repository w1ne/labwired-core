//! The CAN dashboard promo story, on the real firmware.
//!
//! Three STM32F103 Blue Pills run the ELFs in `examples/f103-can-dashboard/
//! prebuilt/` on one timed `can_bus` (the world in `world-fixed.yaml` /
//! `world-bug.yaml`):
//!
//! - ENGINE reads a throttle potentiometer on PA0 and sends 0x100 every 10 ms,
//! - DASHBOARD shows RPM, a bar and the temperature on an SSD1306 and prints a
//!   status line on USART1,
//! - SENSOR sends 0x200 every 100 ms and blinks PC13.
//!
//! With the sensor built for 500 kbit/s everything is ok and the dashboard
//! shows the temperature. With the sensor built for 250 kbit/s (one wrong
//! prescaler) its frames are error frames, it goes error-passive and then
//! bus-off, and the engine's traffic is ok again afterwards.
//!
//! Nothing here pokes a peripheral register: the test only moves the throttle
//! (a system input), reads the OLED framebuffer, the UART, the GPIO output
//! latch and ESR the way a probe would.
//!
//! Release only; `required-features` in Cargo.toml supplies
//! `event-scheduler`. Without the scheduler a timed-CAN world
//! walks every node each cycle (about 190 s per simulated second here).
//!
//! Run: `cargo test --release -p labwired-core --features event-scheduler
//! --test world_can_dashboard_demo`.

#![cfg(not(debug_assertions))]

use labwired_config::EnvironmentManifest;
use labwired_core::network::timed_can::{CanFrameRecord, CanTxOutcome};
use labwired_core::world::{MachineTrait, World};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const MS: u64 = 1_000_000_000;
const US: u64 = 1_000_000;
const CAN_ESR: u64 = 0x4000_6418;
const GPIOC_ODR: u64 = 0x4001_100C;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

type Uart = Arc<Mutex<Vec<u8>>>;

struct Demo {
    world: World,
    engine_uart: Uart,
    dash_uart: Uart,
    sensor_uart: Uart,
    frames: Vec<CanFrameRecord>,
}

impl Demo {
    fn new(env: &str) -> Self {
        let path = repo_root().join("examples/f103-can-dashboard").join(env);
        let manifest: EnvironmentManifest =
            serde_yaml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let mut world = World::from_manifest(manifest, &repo_root()).unwrap();
        let mut sink = |id: &str| -> Uart {
            let s = Uart::default();
            world
                .machines
                .get_mut(id)
                .unwrap()
                .attach_uart_tx_sink(s.clone(), false)
                .unwrap();
            s
        };
        let engine_uart = sink("engine");
        let dash_uart = sink("dashboard");
        let sensor_uart = sink("sensor");
        Demo {
            world,
            engine_uart,
            dash_uart,
            sensor_uart,
            frames: Vec::new(),
        }
    }

    fn node(&mut self, id: &str) -> &mut dyn MachineTrait {
        self.world.machines.get_mut(id).unwrap().as_mut()
    }

    fn now(&self) -> u64 {
        self.world.round_now_ps().unwrap()
    }

    fn run_to(&mut self, ps: u64) {
        let r = self.world.run_until_ps(ps).unwrap();
        assert!(r.values().all(Result::is_ok), "{r:?}");
    }

    /// Run for `ps` more, in `step` slices, calling `probe` after each.
    fn run_for(&mut self, ps: u64, step: u64, mut probe: impl FnMut(&mut Demo)) {
        let end = self.now() + ps;
        while self.now() < end {
            let t = (self.now() + step).min(end);
            self.run_to(t);
            probe(self);
        }
    }

    /// Fold the bus log into `self.frames` (the log is a bounded ring).
    fn collect_frames(&mut self) {
        let rep = &self.world.can_bus_reports()[0];
        assert_eq!(rep.frames_dropped, 0, "the frame ring overflowed");
        self.frames = rep.frames.clone();
    }

    fn throttle(&mut self, percent: f64) {
        self.node("engine")
            .set_input_channel("position", percent)
            .unwrap();
    }

    fn esr(&mut self, id: &str) -> u32 {
        let m = self.node(id);
        u32::from_le_bytes(std::array::from_fn(|b| {
            m.read_u8(CAN_ESR + b as u64).unwrap()
        }))
    }

    fn oled(&mut self) -> Vec<u8> {
        let art = self
            .node("dashboard")
            .display_artifact("oled", true)
            .expect("the dashboard has an OLED");
        let bytes = art.bytes.expect("framebuffer bytes");
        assert_eq!(bytes.len(), 1024);
        bytes
    }

    fn uart(u: &Uart) -> String {
        String::from_utf8_lossy(&u.lock().unwrap()).into_owned()
    }

    /// PC13 output latch (the LED), sampled; counts level changes.
    fn led_toggles(&mut self, window_ps: u64) -> u32 {
        let mut last = None;
        let mut toggles = 0;
        self.run_for(window_ps, 5 * MS, |d| {
            let odr = d.node("sensor").read_u8(GPIOC_ODR + 1).unwrap();
            let level = odr & (1 << 5) != 0; // bit 13
            if last.is_some_and(|l| l != level) {
                toggles += 1;
            }
            last = Some(level);
        });
        toggles
    }
}

// ---- reading the OLED back with the firmware's own font ----------------------

/// (char, 5 columns) pairs parsed from the firmware's `font.h`.
fn font() -> Vec<(char, [u8; 5])> {
    let src =
        std::fs::read_to_string(repo_root().join("examples/f103-can-dashboard/firmware/font.h"))
            .unwrap();
    src.lines()
        .filter(|l| l.trim_start().starts_with('{'))
        .map(|l| {
            let body = &l[l.find('{').unwrap() + 1..l.find('}').unwrap()];
            let cols: Vec<u8> = body
                .split(',')
                .map(|h| u8::from_str_radix(h.trim().trim_start_matches("0x"), 16).unwrap())
                .collect();
            let c = l.split('\'').nth(1).unwrap().chars().next().unwrap();
            (c, cols.try_into().unwrap())
        })
        .collect()
}

/// Column `x` of pixel rows `[page*8, page*8 + 8*vs)` as vs*8 bits.
fn column(fb: &[u8], page: usize, x: usize, vs: usize) -> u16 {
    (0..vs).fold(0u16, |acc, p| {
        acc | (u16::from(fb[(page + p) * 128 + x]) << (8 * p))
    })
}

/// Undo the firmware's glyph scaling: each glyph column repeated `hs` times,
/// each row doubled when `vs == 2`.
fn glyph_column(src: u8, vs: usize) -> u16 {
    if vs == 1 {
        return u16::from(src);
    }
    (0..8).fold(0u16, |acc, i| {
        if src >> i & 1 == 1 {
            acc | 3 << (2 * i)
        } else {
            acc
        }
    })
}

/// Read `n` characters of text starting at (`page`, column 0).
fn read_text(fb: &[u8], page: usize, hs: usize, vs: usize, n: usize) -> String {
    let font = font();
    (0..n)
        .map(|i| {
            let x0 = i * 6 * hs;
            font.iter()
                .find(|(_, g)| {
                    (0..6).all(|col| {
                        let want = if col < 5 { glyph_column(g[col], vs) } else { 0 };
                        (0..hs).all(|r| column(fb, page, x0 + col * hs + r, vs) == want)
                    })
                })
                .map_or('?', |(c, _)| *c)
        })
        .collect()
}

fn rpm_line(fb: &[u8]) -> String {
    read_text(fb, 0, 2, 2, 8)
}
fn temp_line(fb: &[u8]) -> String {
    read_text(fb, 5, 1, 2, 11)
}
fn status_line(fb: &[u8]) -> String {
    read_text(fb, 7, 1, 1, 12).trim_end().to_string()
}
/// Filled columns inside the bar frame.
fn bar_fill(fb: &[u8]) -> usize {
    (1..127).filter(|&x| fb[3 * 128 + x] == 0xFF).count()
}

fn rpm_from(f: &CanFrameRecord) -> u32 {
    u32::from_str_radix(&f.data[..4], 16).unwrap()
}

/// What the engine firmware computes for a conversion of `adc`.
fn rpm_for_adc(adc: u32) -> u32 {
    800 + adc * 6200 / 4095
}

fn engine_frames(frames: &[CanFrameRecord]) -> Vec<&CanFrameRecord> {
    frames.iter().filter(|f| f.from == "engine").collect()
}

// ---- the story -----------------------------------------------------------------

#[test]
fn with_the_fixed_sensor_every_frame_is_ok_and_the_dashboard_follows_the_throttle() {
    let mut d = Demo::new("world-fixed.yaml");

    // Idle throttle: 0 % -> ADC 0 -> 800 rpm.
    d.throttle(0.0);
    d.run_to(300 * MS);
    let fb = d.oled();
    assert_eq!(rpm_line(&fb), "RPM  800");
    assert_eq!(bar_fill(&fb), 0);
    assert_eq!(status_line(&fb), "CAN OK");
    let temp = temp_line(&fb);
    assert!(
        temp.starts_with("TEMP 2") && temp.ends_with(" C") && temp.contains('.'),
        "the dashboard shows the sensor's temperature: {temp:?}"
    );

    // Mid and full throttle: the display and the bar follow within 2 frames.
    for (percent, adc) in [(50.0, 2047u32), (100.0, 4095)] {
        d.throttle(percent);
        d.run_for(300 * MS, 50 * MS, |_| {});
        let fb = d.oled();
        let rpm = rpm_for_adc(adc);
        assert_eq!(
            rpm_line(&fb),
            format!("RPM {rpm:>4}"),
            "throttle {percent} %"
        );
        assert_eq!(bar_fill(&fb), ((rpm - 800) * 126 / 6200) as usize);
    }
    assert_eq!(rpm_line(&d.oled()), "RPM 7000");
    assert_eq!(bar_fill(&d.oled()), 126);

    // Let a UART second elapse, then check the whole bus.
    d.run_to(1300 * MS);
    d.collect_frames();
    let rep = &d.world.can_bus_reports()[0];
    assert!(
        d.frames.iter().all(|f| f.result == CanTxOutcome::Ok),
        "no error, no_ack or lost frame: {:?}",
        rep.totals
    );
    assert!(rep.nodes.iter().all(|n| n.tec == 0 && n.rec == 0));
    let engine = engine_frames(&d.frames);
    assert!(
        engine.len() >= 125,
        "{} engine frames in 1.3 s",
        engine.len()
    );
    assert!(d.frames.iter().filter(|f| f.from == "sensor").count() >= 10);
    // The throttle moved the engine's frames too (800 .. 7000 rpm on the wire).
    assert_eq!(rpm_from(engine[engine.len() / 5]), 800);
    assert_eq!(rpm_from(engine.last().unwrap()), 7000);

    let dash = Demo::uart(&d.dash_uart);
    assert!(
        dash.contains("DASH rpm=7000") && dash.contains("can=OK tec=0 rec=0"),
        "{dash:?}"
    );
    assert!(
        !dash.contains("rx200=0"),
        "the dashboard counted 0x200 frames: {dash:?}"
    );
    let sensor = Demo::uart(&d.sensor_uart);
    assert!(
        sensor.contains("SENSOR bitrate=500000 state=ACTIVE tec=0 rec=0"),
        "{sensor:?}"
    );
    assert!(Demo::uart(&d.engine_uart).is_empty());
    // Healthy: the LED blinks slowly (1 Hz = 2 changes a second).
    let toggles = d.led_toggles(1000 * MS);
    assert!((1..=3).contains(&toggles), "{toggles} LED changes in 1 s");
}

#[test]
fn with_the_250k_sensor_it_goes_passive_then_bus_off_and_the_engine_traffic_recovers() {
    let mut d = Demo::new("world-bug.yaml");
    d.throttle(80.0);

    // Sample the sensor's ESR every 100 us across the first 150 ms.
    let (mut passive_at, mut off_at) = (None, None);
    let mut tec_while_active = false;
    d.run_for(150 * MS, 100 * US, |d| {
        let esr = d.esr("sensor");
        let t = d.now();
        if esr & 0b010 != 0 && passive_at.is_none() {
            passive_at = Some(t);
        }
        if esr & 0b100 != 0 && off_at.is_none() {
            off_at = Some(t);
        }
        tec_while_active |= esr & 0b010 == 0 && (esr >> 24 != 0 || (esr >> 16) & 0xFF != 0);
    });
    let (passive_at, off_at) = (passive_at.expect("passive"), off_at.expect("bus-off"));
    // The wrong-rate node sees the engine's first frame at 10 ms as garbage and,
    // error-active, destroys it with active error flags (REC +8 each, ISO 11898-1
    // 6.x) until it is error-passive after 16 of them, at ~11 ms. It only
    // transmits at 100 ms, where each failed attempt adds 8 to TEC until bus-off.
    assert!(
        (10 * MS..13 * MS).contains(&passive_at),
        "passive at {passive_at} ps"
    );
    assert!(
        (100 * MS..105 * MS).contains(&off_at),
        "bus-off at {off_at} ps"
    );
    assert!(
        tec_while_active,
        "the counters climbed while still error-active"
    );

    // Change the throttle after the fault; the dashboard must still follow it.
    d.throttle(20.0);
    d.run_to(1500 * MS);
    d.collect_frames();
    let frames = d.frames.clone();
    let rep = d.world.can_bus_reports()[0].clone();

    // Sensor: stays bus-off (ABOM off), never gets a frame through.
    let esr = d.esr("sensor");
    assert_eq!(esr & 0b111, 0b111, "BOFF | EPVF | EWGF");
    assert_eq!(
        (esr >> 16) & 0xFF,
        248,
        "TEC 248: the overflowing +8 is not stored"
    );
    let sensor_frames: Vec<_> = frames.iter().filter(|f| f.from == "sensor").collect();
    assert!(
        sensor_frames.len() >= 20,
        "{} attempts",
        sensor_frames.len()
    );
    assert!(sensor_frames
        .iter()
        .all(|f| f.result == CanTxOutcome::Error));
    assert!(sensor_frames
        .iter()
        .all(|f| f.t_ps >= 100 * MS && f.t_ps < off_at));

    // Engine: its first frames were destroyed and retried automatically; once
    // the sensor is passive every frame is ok, and it never left error-active.
    let engine = engine_frames(&frames);
    let failed: Vec<_> = engine
        .iter()
        .filter(|f| f.result != CanTxOutcome::Ok)
        .collect();
    assert!(
        (10..=16).contains(&failed.len()),
        "{} engine frames destroyed",
        failed.len()
    );
    assert!(failed.iter().all(|f| f.result == CanTxOutcome::Error));
    assert!(failed.iter().all(|f| f.t_ps < passive_at));
    let delivered: Vec<_> = engine
        .iter()
        .filter(|f| f.result == CanTxOutcome::Ok)
        .collect();
    let first_ok = delivered[0];
    assert!(
        first_ok.t_ps - 10 * MS > 500 * US && first_ok.t_ps - 10 * MS < 2 * MS,
        "the dashboard's first RPM frame came {} us late",
        (first_ok.t_ps - 10 * MS) / US
    );
    assert!(delivered
        .iter()
        .skip(1)
        .all(|f| f.t_ps > passive_at - 200 * US));
    assert!(
        delivered.len() >= 135,
        "{} engine frames delivered",
        delivered.len()
    );
    // Steady 10 ms spacing from then on, including through the sensor's
    // bus-off (the longest gap is one frame period plus the sensor's burst).
    let gap = delivered
        .windows(2)
        .map(|w| w[1].t_ps - w[0].t_ps)
        .max()
        .unwrap();
    assert!(gap <= 13 * MS, "max gap between RPM frames {} us", gap / US);
    let engine_node = rep.nodes.iter().find(|n| n.node == "engine").unwrap();
    assert_eq!(
        format!("{:?}", engine_node.state),
        "Active",
        "the healthy nodes stay error-active"
    );
    assert_eq!(
        rep.nodes
            .iter()
            .find(|n| n.node == "dashboard")
            .unwrap()
            .tec,
        0
    );

    // Dashboard: still shows RPM, follows the throttle, no temperature, and
    // its own controller is healthy.
    let fb = d.oled();
    assert_eq!(rpm_line(&fb), format!("RPM {:>4}", rpm_for_adc(819)));
    assert_eq!(
        bar_fill(&fb),
        ((rpm_for_adc(819) - 800) * 126 / 6200) as usize
    );
    assert_eq!(temp_line(&fb).trim_end(), "TEMP --");
    assert_eq!(status_line(&fb), "CAN OK");
    let dash = Demo::uart(&d.dash_uart);
    assert!(
        dash.contains("rx200=0") && dash.contains("can=OK"),
        "{dash:?}"
    );
    assert!(dash.contains("rx100="), "{dash:?}");
    assert!(rep.totals.error as usize >= failed.len() + sensor_frames.len());

    // Sensor firmware reports itself, and its LED blinks fast (5 Hz).
    let sensor = Demo::uart(&d.sensor_uart);
    assert!(
        sensor.contains("SENSOR bitrate=250000 state=BUS_OFF tec=248"),
        "{sensor:?}"
    );
    let toggles = d.led_toggles(1000 * MS);
    assert!(toggles >= 8, "{toggles} LED changes in 1 s");
}

/// The promo needs the three-node world to run at least in real time. Run:
/// `cargo test --release -p labwired-core --features event-scheduler --test
/// world_can_dashboard_demo -- --ignored --nocapture`.
#[test]
#[ignore = "wall-clock probe; run with --release -- --ignored --nocapture"]
fn the_demo_world_runs_faster_than_real_time() {
    for env in ["world-fixed.yaml", "world-bug.yaml"] {
        let mut d = Demo::new(env);
        d.throttle(50.0);
        let started = std::time::Instant::now();
        d.run_for(1000 * MS, 20 * MS, |_| {});
        let wall = started.elapsed().as_secs_f64();
        println!(
            "{env}: 1 s simulated in {wall:.3} s wall = {:.1}x real time",
            1.0 / wall
        );
        assert!(wall < 1.0, "{env} is slower than real time: {wall:.2} s");
    }
}
