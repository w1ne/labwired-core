// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
//! SmartBasket handle POC firmware — the clip-on checkout handle.
//!
//! A standard shopping basket carries this handle: an ESP32-S3 with a
//! downward-facing OV3660 camera (DVP), an ADXL345 accelerometer, an HX711
//! load-cell ADC and an INA219 battery gauge. The device is a sensor sled:
//! it captures event-triggered JPEG bursts with weight and motion context and
//! dumps the session to the checkout station. Recognition happens at the
//! station/cloud, never here.
//!
//! Pipeline (all integer math; the simulator models no Xtensa FPU):
//!   1. Tare: the first load-cell samples establish the empty-basket baseline
//!      (debris/papers inside are zeroed out).
//!   2. Event: the accelerometer stream and the weight stream run together.
//!      A weight change that settles at ≥ 40 g from the last captured weight
//!      triggers a capture burst; raw acceleration without a weight change is
//!      logged as `ACCEL` and ignored (basket jostle, walking).
//!   3. Capture: 2 JPEG frames (160×120, quality 12) through LCD_CAM + GDMA,
//!      stored with per-frame CRC-32, weight and accel-peak context.
//!   4. Battery: the INA219 bus voltage is the pack terminal; low/critical
//!      thresholds (3.30 V / 3.20 V) log `LOW_BATT` / `CRIT_BATT`, and a
//!      critical pack stops new captures (`CRIT_SKIP`).
//!   5. Checkout: DOCK (GPIO48) rising = handle placed on a station; the
//!      firmware dumps every event and frame over USB-Serial-JTAG
//!      (`DUMP`/`FRAME`/`H <hex>`/`ENDFRAME`/`DUMP end`). DOCK falling resets
//!      the session for the next shopper.
//!
//! Pins: camera SCCB I2C1 (SDA GPIO4 / SCL GPIO5) + DVP on LCD_CAM; IMU and
//! gauge on I2C0 (SDA GPIO1 / SCL GPIO2); HX711 DT GPIO21 / SCK GPIO47; DOCK
//! input GPIO48. See system.yaml for the board wiring.

#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::{
    gpio::{Input, InputConfig, Level, Output, OutputConfig},
    i2c::master::{Config as I2cConfig, I2c},
    main,
    time::Instant,
    Blocking,
};
use esp_println::println;

// ── I2C slaves ──────────────────────────────────────────────────────────────
const OV3660_SCCB_ADDR: u8 = 0x3C;
const ADXL345_ADDR: u8 = 0x53;
const INA219_ADDR: u8 = 0x40;

// ── LCD_CAM + GDMA registers (ESP32-S3 TRM) ─────────────────────────────────
const LCD_CAM_BASE: u32 = 0x6004_1000;
const CAM_CTRL1: u32 = LCD_CAM_BASE + 0x08;
const LC_DMA_INT_ENA: u32 = LCD_CAM_BASE + 0x64;
const LC_DMA_INT_RAW: u32 = LCD_CAM_BASE + 0x68;
const LC_DMA_INT_CLR: u32 = LCD_CAM_BASE + 0x70;
const CAM_START: u32 = 1 << 29;
const CAM_RESET: u32 = 1 << 30;
const INT_CAM_VSYNC: u32 = 1 << 2;

const GDMA_BASE: u32 = 0x6003_F000;
const IN_CONF0: u32 = GDMA_BASE;
const IN_CONF1: u32 = GDMA_BASE + 0x04;
const IN_INT_RAW: u32 = GDMA_BASE + 0x08;
const IN_INT_ENA: u32 = GDMA_BASE + 0x10;
const IN_INT_CLR: u32 = GDMA_BASE + 0x14;
const IN_LINK: u32 = GDMA_BASE + 0x20;
const IN_PERI_SEL: u32 = GDMA_BASE + 0x48;
const IN_RST: u32 = 1 << 0;
const IN_LINK_STOP: u32 = 1 << 21;
const IN_LINK_START: u32 = 1 << 22;
const IN_SUC_EOF: u32 = 1 << 1;
const PERI_SEL_LCD_CAM: u32 = 5;

#[inline(always)]
unsafe fn rd(addr: u32) -> u32 {
    core::ptr::read_volatile(addr as *const u32)
}

#[inline(always)]
unsafe fn wr(addr: u32, v: u32) {
    core::ptr::write_volatile(addr as *mut u32, v)
}

// ── Frame ring: 4 descriptors x 1 KiB over a 4 KiB buffer ───────────────────
const JPEG_BLOCK: u32 = 1024;
const JPEG_NODES: usize = 4;
const FRAME_MAX: usize = 4096;

#[repr(align(16))]
struct DescRing([u32; JPEG_NODES * 4]);
#[repr(align(16))]
struct FrameBuf([u32; 1024]); // 4 KiB

static mut DESC_RING: DescRing = DescRing([0; JPEG_NODES * 4]);
static mut FRAME_BUF: FrameBuf = FrameBuf([0; 1024]);

fn desc_addr() -> u32 {
    unsafe { (&raw const DESC_RING.0) as *const u32 as u32 }
}

fn buf_addr() -> u32 {
    unsafe { (&raw const FRAME_BUF.0) as *const u32 as u32 }
}

// ── SCCB (I2C1, 16-bit register address) ────────────────────────────────────
fn sccb_write(i2c: &mut I2c<'_, Blocking>, reg: u16, v: u8) {
    let _ = i2c.write(OV3660_SCCB_ADDR, &[(reg >> 8) as u8, reg as u8, v]);
}

fn sccb_read(i2c: &mut I2c<'_, Blocking>, reg: u16) -> u8 {
    let mut b = [0u8; 1];
    let _ = i2c.write_read(OV3660_SCCB_ADDR, &[(reg >> 8) as u8, reg as u8], &mut b);
    b[0]
}

/// Program the window + JPEG format the way `esp32-camera` does.
fn cam_program(i2c: &mut I2c<'_, Blocking>, w: u16, h: u16) {
    sccb_write(i2c, 0x3008, 0x82); // software reset
    sccb_write(i2c, 0x3808, (w >> 8) as u8);
    sccb_write(i2c, 0x3809, w as u8);
    sccb_write(i2c, 0x380A, (h >> 8) as u8);
    sccb_write(i2c, 0x380B, h as u8);
    sccb_write(i2c, 0x4300, 0x30); // YUV422 core + compression path
    sccb_write(i2c, 0x3821, 0x20); // JPEG enable
    sccb_write(i2c, 0x4407, 12); // quality 12
}

/// `ll_cam_dma_init` + `allocate_dma_descriptors`: a DMA-owned ring.
unsafe fn cam_arm_ring() {
    for n in 0..JPEG_NODES {
        let d = desc_addr() + (n as u32) * 16;
        let next = desc_addr() + (((n + 1) % JPEG_NODES) as u32) * 16;
        wr(d, (1 << 31) | JPEG_BLOCK);
        wr(d + 4, buf_addr() + (n as u32) * JPEG_BLOCK);
        wr(d + 8, next);
    }
    wr(IN_INT_CLR, !0);
    wr(IN_INT_ENA, 0);
    wr(IN_CONF0, IN_RST);
    wr(IN_CONF0, 0);
    wr(IN_CONF1, 0); // IN_CHECK_OWNER clear
    wr(IN_PERI_SEL, PERI_SEL_LCD_CAM);
}

/// `ll_cam_start` for the JPEG half-buffer size.
unsafe fn cam_start() {
    let ctrl1 = rd(CAM_CTRL1) & !CAM_START;
    wr(CAM_CTRL1, ctrl1);
    wr(IN_INT_CLR, !0);
    wr(IN_INT_ENA, IN_SUC_EOF);
    wr(IN_CONF0, IN_RST);
    wr(IN_CONF0, 0);
    wr(CAM_CTRL1, (JPEG_BLOCK - 1) | CAM_RESET);
    wr(IN_LINK, (desc_addr() & 0xF_FFFF) | IN_LINK_START);
    wr(CAM_CTRL1, (JPEG_BLOCK - 1) | CAM_START);
}

/// Wait up to `timeout_ms` of guest time for the raw VSYNC flag.
unsafe fn wait_vsync(timeout_ms: u64) -> bool {
    let t0 = Instant::now();
    while rd(LC_DMA_INT_RAW) & INT_CAM_VSYNC == 0 {
        if t0.elapsed().as_millis() > timeout_ms {
            return false;
        }
    }
    true
}

unsafe fn cam_stop() {
    wr(IN_INT_ENA, 0);
    wr(IN_INT_CLR, IN_SUC_EOF);
    let ctrl1 = rd(CAM_CTRL1) & !CAM_START;
    wr(CAM_CTRL1, ctrl1);
    wr(IN_LINK, IN_LINK_STOP);
}

/// One blocking JPEG capture. Returns the JPEG length as `cam_hal` computes
/// it: (EOFs + 1) blocks, with EOI searched forward in the last block + 1.
/// The capture copy lands in `out`; returns the length, 0 on timeout.
unsafe fn cam_capture(out: &mut [u8; FRAME_MAX]) -> usize {
    // The sensor streams continuously once CAM_START is set. A raw VSYNC may
    // already be pending from an earlier frame, and the edge right after the
    // interrupt is cleared may be the CLOSE of a frame that was in flight
    // when we started. Drain one edge, then arm the ring on the following
    // VSYNC (the frame's opening) and capture until the one after it closes
    // the frame — the exact open/close contract `cam_hal` counts.
    wr(CAM_CTRL1, 4096 - 1 | CAM_START);
    wr(LC_DMA_INT_ENA, INT_CAM_VSYNC);
    if !wait_vsync(2000) {
        return 0;
    }
    wr(LC_DMA_INT_CLR, INT_CAM_VSYNC);
    if !wait_vsync(2000) {
        return 0;
    }
    wr(LC_DMA_INT_CLR, INT_CAM_VSYNC);
    cam_start();

    let mut eofs = 0u32;
    let t1 = Instant::now();
    loop {
        if rd(IN_INT_RAW) & IN_SUC_EOF != 0 {
            wr(IN_INT_CLR, IN_SUC_EOF);
            eofs += 1;
        }
        if rd(LC_DMA_INT_RAW) & INT_CAM_VSYNC != 0 {
            cam_stop();
            break;
        }
        if t1.elapsed().as_millis() > 2000 || eofs >= JPEG_NODES as u32 {
            cam_stop();
            return 0;
        }
    }

    let reported = ((eofs + 1) as usize) * JPEG_BLOCK as usize;
    let bytes = core::slice::from_raw_parts((&raw const FRAME_BUF.0) as *const u8, reported);
    let window = reported.saturating_sub(JPEG_BLOCK as usize + 1);
    let mut len = 0;
    for i in window..reported.saturating_sub(1) {
        if bytes[i] == 0xFF && bytes[i + 1] == 0xD9 {
            len = i + 2;
            break;
        }
    }
    if len == 0 || len > FRAME_MAX {
        return 0;
    }
    out[..len].copy_from_slice(&bytes[..len]);
    len
}

/// CRC-32 (IEEE 802.3, reflected) computed bitwise — frames carry their own
/// integrity tag so the station can verify a dump end to end.
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

// ── ADXL345 accelerometer ───────────────────────────────────────────────────
/// Read the three axes in milli-g (full resolution: 256 counts/g).
fn imu_read_mg(i2c: &mut I2c<'_, Blocking>) -> (i32, i32, i32) {
    let mut a = [0u8; 6];
    if i2c.write_read(ADXL345_ADDR, &[0x32], &mut a).is_err() {
        return (0, 0, 0);
    }
    let ax = i16::from_le_bytes([a[0], a[1]]) as i32;
    let ay = i16::from_le_bytes([a[2], a[3]]) as i32;
    let az = i16::from_le_bytes([a[4], a[5]]) as i32;
    let mg = |c: i32| c * 1000 / 256;
    (mg(ax), mg(ay), mg(az))
}

// ── HX711 load-cell ADC ─────────────────────────────────────────────────────
/// Bit-bang one 25-pulse frame (24 data bits MSB first); returns raw counts.
fn hx711_read(dt: &Input, sck: &mut Output) -> i32 {
    let t0 = Instant::now();
    while dt.is_high() {
        if t0.elapsed().as_millis() > 200 {
            return 0;
        }
    }
    let mut v: u32 = 0;
    for _ in 0..24 {
        sck.set_high();
        let bit = dt.is_high() as u32;
        sck.set_low();
        v = (v << 1) | bit;
    }
    sck.set_high();
    sck.set_low();
    if v & 0x0080_0000 != 0 {
        (v | 0xFF00_0000) as i32
    } else {
        v as i32
    }
}

// ── INA219 battery gauge ────────────────────────────────────────────────────
/// Pack terminal in millivolts (bus-voltage register: 4 mV/LSB at bit 3).
fn batt_read_mv(i2c: &mut I2c<'_, Blocking>) -> u32 {
    let mut b = [0u8; 2];
    if i2c.write_read(INA219_ADDR, &[0x02], &mut b).is_err() {
        return 0;
    }
    ((u16::from_be_bytes(b) >> 3) as u32) * 4
}

// ── Session store ───────────────────────────────────────────────────────────
const MAX_EVENTS: usize = 8;
const FRAMES_PER_EVENT: usize = 2;
const MAX_FRAMES: usize = MAX_EVENTS * FRAMES_PER_EVENT;

/// Battery thresholds in mV. 3.30 V is the pack coming off its flat region;
/// 3.20 V is where a POC pack should stop opening new sessions.
const BATT_LOW_MV: u32 = 3300;
const BATT_CRIT_MV: u32 = 3200;

/// Weight-trigger threshold and settle window, in mg and milliseconds.
const TRIGGER_MG: i32 = 40_000;
/// Wait for the filter to come within this of the raw sample before the
/// capture (the weight moves smoothly in the real world; the cap keeps a
/// noisy basket from stalling a capture forever).
const CONVERGE_TOL_MG: i32 = 20_000;
const CONVERGE_CAP_MS: u64 = 1500;
const SETTLE_TOL_MG: i32 = 25_000;
const SETTLE_SAMPLES: u32 = 2;
const MIN_CAPTURE_GAP_MS: u64 = 400;

/// Raw acceleration deviation from 1 g that counts as motion (squared mg).
const MOTION_DEV_SQ: i32 = 400_000; // ≈0.2 g band on |a|² at ~1 g

#[derive(Clone, Copy)]
struct Event {
    t_ms: u32,
    weight_g: i32,
    delta_g: i32,
    accel_peak_mg: i32,
    len: [u16; FRAMES_PER_EVENT],
    crc: [u32; FRAMES_PER_EVENT],
}

#[derive(Clone, Copy)]
struct FrameBytes {
    len: u16,
    data: [u8; FRAME_MAX],
}

struct Session {
    events: [Event; MAX_EVENTS],
    count: usize,
    frames: [FrameBytes; MAX_FRAMES],
    frame_count: usize,
    dumped: bool,
}

static mut SESSION: Session = Session {
    events: [Event {
        t_ms: 0,
        weight_g: 0,
        delta_g: 0,
        accel_peak_mg: 0,
        len: [0; FRAMES_PER_EVENT],
        crc: [0; FRAMES_PER_EVENT],
    }; MAX_EVENTS],
    count: 0,
    frames: [FrameBytes {
        len: 0,
        data: [0; FRAME_MAX],
    }; MAX_FRAMES],
    frame_count: 0,
    dumped: false,
};

unsafe fn session_reset() {
    SESSION.count = 0;
    SESSION.frame_count = 0;
    SESSION.dumped = false;
}

/// Dump the session over the host link: one `FRAME` header line per frame,
/// the JPEG bytes as `H <hex>` lines, `ENDFRAME`, then `DUMP end`.
unsafe fn session_dump() {
    let s = &*core::ptr::addr_of!(SESSION);
    println!("DUMP events={} frames={}", s.count, s.frame_count);
    for (i, e) in s.events[..s.count].iter().enumerate() {
        println!(
            "EVENT i={} t={} w={} dw={} a={} f0len={} f0crc=0x{:08x} f1len={} f1crc=0x{:08x}",
            i,
            e.t_ms,
            e.weight_g,
            e.delta_g,
            e.accel_peak_mg,
            e.len[0],
            e.crc[0],
            e.len[1],
            e.crc[1]
        );
    }
    for i in 0..s.frame_count {
        let f = &s.frames[i];
        let len = f.len as usize;
        // Hex, not raw bytes: the USB-Serial-JTAG printer drops the tail of a
        // slice when its FIFO is still draining, so an ASCII stream that can
        // re-sync on newlines is the robust format for a bulk dump.
        println!(
            "FRAME i={} len={} crc=0x{:08x}",
            i,
            len,
            crc32(&f.data[..len])
        );
        let mut off = 0;
        while off < len {
            let n = core::cmp::min(16, len - off);
            let mut line = [0u8; 32];
            for (j, &b) in f.data[off..off + n].iter().enumerate() {
                line[j * 2] = HEX[(b >> 4) as usize];
                line[j * 2 + 1] = HEX[(b & 0xF) as usize];
            }
            let text = core::str::from_utf8_unchecked(&line[..n * 2]);
            println!("H {}", text);
            off += n;
        }
        println!("ENDFRAME i={}", i);
    }
    println!("DUMP end");
}

const HEX: [u8; 16] = *b"0123456789abcdef";

#[main]
fn main() -> ! {
    let p = esp_hal::init(esp_hal::Config::default());
    println!("BOOT smart-basket-handle");

    // I2C0: IMU + battery gauge (SDA GPIO1 / SCL GPIO2).
    let mut i2c0 = I2c::new(p.I2C0, I2cConfig::default())
        .unwrap()
        .with_sda(p.GPIO1)
        .with_scl(p.GPIO2);

    // I2C1: camera SCCB (SIOD GPIO4 / SIOC GPIO5 — the board's camera pads).
    let mut i2c1 = I2c::new(p.I2C1, I2cConfig::default())
        .unwrap()
        .with_sda(p.GPIO4)
        .with_scl(p.GPIO5);

    // HX711 bit-bang pins and the DOCK contact.
    let dt = Input::new(p.GPIO21, InputConfig::default());
    let mut sck = Output::new(p.GPIO47, Level::Low, OutputConfig::default());
    let dock = Input::new(p.GPIO48, InputConfig::default());

    // Camera bring-up: probe, program window/format, arm the DMA ring.
    let pid_h = sccb_read(&mut i2c1, 0x300A);
    let pid_l = sccb_read(&mut i2c1, 0x300B);
    println!("CAM pid=0x{:02x}{:02x}", pid_h, pid_l);
    cam_program(&mut i2c1, 160, 120);
    unsafe {
        cam_arm_ring();
    }

    // IMU measure mode.
    let _ = i2c0.write(ADXL345_ADDR, &[0x2D, 0x08]);

    // Battery.
    let mut batt_mv = batt_read_mv(&mut i2c0);
    let soc = |mv: u32| -> i32 {
        let mv = mv as i32;
        (((mv - 3300) * 100) / 900).clamp(0, 100)
    };
    println!("BATT mv={} soc={}", batt_mv, soc(batt_mv));
    let mut batt_low = false;
    let mut batt_crit = false;
    let mut last_batt_print = batt_mv;

    // Tare: the empty-basket baseline (debris inside included).
    let mut tare_sum: i64 = 0;
    for _ in 0..5 {
        tare_sum += hx711_read(&dt, &mut sck) as i64;
    }
    let tare_counts = (tare_sum / 5) as i32;
    println!("TARE tare_g={}", tare_counts / 100);

    // Pipeline state.
    let mut filt_mg: i32 = tare_counts * 10;
    let mut prev_mg: i32 = filt_mg;
    let mut settled_mg: i32 = filt_mg;
    let mut last_captured_mg: i32 = filt_mg;
    let mut stable_samples: u32 = 0;
    let mut accel_peak_mg: i32 = 0;
    let mut last_accel_print = Instant::now() - esp_hal::time::Duration::from_millis(1000);
    let mut next_accel = Instant::now();
    let mut next_weight = Instant::now();
    let mut next_batt = Instant::now() + esp_hal::time::Duration::from_millis(500);
    let mut last_capture = Instant::now() - esp_hal::time::Duration::from_millis(60_000);
    let mut pending = false;
    let mut pending_from_mg: i32 = 0;
    let mut pending_deadline = Instant::now();
    let mut dock_prev = false;
    let t_start = Instant::now();

    loop {
        let now = Instant::now();

        // Accelerometer: 40 Hz poll, deviation from 1 g in squared mg.
        if now >= next_accel {
            next_accel = now + esp_hal::time::Duration::from_millis(25);
            let (ax, ay, az) = imu_read_mg(&mut i2c0);
            let mag_sq = ax * ax + ay * ay + az * az;
            let dev = (mag_sq - 1_000_000).abs();
            if dev > MOTION_DEV_SQ {
                let dev_mg = if mag_sq > 1_000_000 {
                    mag_sq - 1_000_000
                } else {
                    1_000_000 - mag_sq
                };
                if dev_mg > accel_peak_mg {
                    accel_peak_mg = dev_mg;
                }
                // One line per motion burst, not one per 25 ms sample.
                if now - last_accel_print >= esp_hal::time::Duration::from_millis(250) {
                    last_accel_print = now;
                    println!("ACCEL t={} dev_sq={}", now.elapsed().as_millis(), dev);
                }
            }
        }

        // Load cell: 10 Hz poll, 4-sample integer average.
        if now >= next_weight {
            next_weight = now + esp_hal::time::Duration::from_millis(100);
            let raw = hx711_read(&dt, &mut sck);
            let w_mg = raw * 10; // 100 counts/g -> 10 mg per count
            filt_mg = (filt_mg * 3 + w_mg) / 4;

            if (filt_mg - prev_mg).abs() <= SETTLE_TOL_MG {
                stable_samples += 1;
            } else {
                stable_samples = 0;
            }
            prev_mg = filt_mg;

            if stable_samples >= SETTLE_SAMPLES {
                settled_mg = filt_mg;
                stable_samples = SETTLE_SAMPLES;
                let delta = settled_mg - last_captured_mg;
                let gap_ok =
                    now - last_capture >= esp_hal::time::Duration::from_millis(MIN_CAPTURE_GAP_MS);
                if !pending {
                    if delta.abs() >= TRIGGER_MG && gap_ok {
                        // A real placement: wait for the filter to converge on
                        // the new weight before capturing, so the recorded
                        // weight tracks ground truth instead of the middle of
                        // the ramp. One second and a half is the cap.
                        pending = true;
                        pending_from_mg = last_captured_mg;
                        pending_deadline =
                            now + esp_hal::time::Duration::from_millis(CONVERGE_CAP_MS);
                    } else if delta.abs() < TRIGGER_MG {
                        // Drift smaller than a placement is absorbed, so the
                        // settle filter's tail can never fake a second item.
                        last_captured_mg = settled_mg;
                    }
                } else {
                    let converged = (w_mg - settled_mg).abs() <= CONVERGE_TOL_MG;
                    if converged || now >= pending_deadline {
                        let delta = settled_mg - pending_from_mg;
                        if batt_crit {
                            println!(
                                "CRIT_SKIP t={} dw={}",
                                now.elapsed().as_millis(),
                                delta / 1000
                            );
                        } else {
                            unsafe {
                                capture_event(now, settled_mg, delta, accel_peak_mg);
                            }
                            accel_peak_mg = 0;
                        }
                        last_captured_mg = settled_mg;
                        last_capture = now;
                        pending = false;
                    }
                }
            }
        }

        // Battery: 1 Hz poll, log on first read and on every 50 mV change.
        if now >= next_batt {
            next_batt = now + esp_hal::time::Duration::from_millis(500);
            batt_mv = batt_read_mv(&mut i2c0);
            if !batt_low && batt_mv < BATT_LOW_MV {
                batt_low = true;
                println!("LOW_BATT mv={} soc={}", batt_mv, soc(batt_mv));
            }
            if !batt_crit && batt_mv < BATT_CRIT_MV {
                batt_crit = true;
                println!("CRIT_BATT mv={} soc={}", batt_mv, soc(batt_mv));
            }
            if (batt_mv as i32 - last_batt_print as i32).abs() >= 50 {
                println!("BATT mv={} soc={}", batt_mv, soc(batt_mv));
                last_batt_print = batt_mv;
            }
        }

        // Checkout dock: rising edge dumps, falling edge resets the session.
        let dock_now = dock.is_high();
        if dock_now && !dock_prev {
            unsafe {
                if !SESSION.dumped {
                    session_dump();
                    SESSION.dumped = true;
                } else {
                    println!("DUMP already");
                }
            }
        } else if !dock_now && dock_prev {
            unsafe {
                session_reset();
            }
            last_captured_mg = settled_mg;
            println!("SESSION reset t={}", t_start.elapsed().as_millis());
        }
        dock_prev = dock_now;

        core::hint::spin_loop();
    }
}

/// Capture a burst for one placement event and file it in the session.
unsafe fn capture_event(now: Instant, weight_mg: i32, delta_mg: i32, accel_peak_mg: i32) {
    let s = &mut *core::ptr::addr_of_mut!(SESSION);
    if s.count >= MAX_EVENTS || s.frame_count + FRAMES_PER_EVENT > MAX_FRAMES {
        println!("EVT_DROP t={}", now.elapsed().as_millis());
        return;
    }
    let ev_idx = s.count;
    let frame_base = s.frame_count;
    let mut lens = [0u16; FRAMES_PER_EVENT];
    let mut crcs = [0u32; FRAMES_PER_EVENT];

    for k in 0..FRAMES_PER_EVENT {
        let frame = &mut s.frames[frame_base + k];
        let len = cam_capture(&mut frame.data);
        if len == 0 {
            println!("CAPTURE_FAIL t={}", now.elapsed().as_millis());
            return;
        }
        frame.len = len as u16;
        lens[k] = len as u16;
        crcs[k] = crc32(&frame.data[..len]);
    }

    s.events[ev_idx] = Event {
        t_ms: now.elapsed().as_millis() as u32,
        weight_g: weight_mg / 1000,
        delta_g: delta_mg / 1000,
        accel_peak_mg,
        len: lens,
        crc: crcs,
    };
    s.count += 1;
    s.frame_count += FRAMES_PER_EVENT;
    println!(
        "EVT i={} t={} w={} dw={} a={} f0={} f1={}",
        ev_idx,
        s.events[ev_idx].t_ms,
        s.events[ev_idx].weight_g,
        s.events[ev_idx].delta_g,
        accel_peak_mg,
        lens[0],
        lens[1]
    );
}
