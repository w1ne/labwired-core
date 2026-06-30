// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// Round-trip fidelity test for the MLX90640 thermal-camera model.
//
// This drives the LabWired `Mlx90640` device model exactly as on-target
// firmware would (16-bit-addressed I²C reads/writes through the `I2cDevice`
// trait), and decodes the result with the REAL, unmodified Melexis MLX90640
// driver (`third_party/mlx90640-library`, compiled into the bridge). It then
// asserts that the decoded per-pixel °C match the injected thermal scene —
// including the hotspot landing on the correct pixel.
//
// Run with:
//   cargo test -p labwired-core --features mlx90640-decode-test --test mlx90640_decode

#![cfg(feature = "mlx90640-decode-test")]

use std::cell::RefCell;
use std::sync::{Mutex, MutexGuard};

use labwired_core::peripherals::components::mlx90640::{Mlx90640, ThermalScene, MLX90640_ADDR};
use labwired_core::peripherals::i2c::I2cDevice;

// The vendored Melexis driver and the bridge use file-scope `static` scratch
// buffers (`paramsMLX90640`, `frameData[834]`), which are not re-entrant across
// threads. Cargo runs tests in parallel, so serialize every decode through this
// lock to keep the C statics single-owner for the duration of a test.
static DECODE_LOCK: Mutex<()> = Mutex::new(());

fn lock_decode() -> MutexGuard<'static, ()> {
    DECODE_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

thread_local! {
    /// The model under test for the current thread. The C driver's I²C shim
    /// calls back into Rust, which drives this model over its I²C interface.
    static MODEL: RefCell<Option<Mlx90640>> = const { RefCell::new(None) };
}

fn with_model<R>(f: impl FnOnce(&mut Mlx90640) -> R) -> R {
    MODEL.with(|m| {
        let mut guard = m.borrow_mut();
        let dev = guard.as_mut().expect("model not installed");
        f(dev)
    })
}

// ── Driver I²C callbacks (route through the real 16-bit protocol) ────────────

/// Read `n` 16-bit words from `start_addr`, driving the model's I²C state
/// machine: write the 2-byte big-endian register address, repeated-start, then
/// stream `2*n` bytes (MSB first), reassembling host-order words.
/// # Safety
/// `out` must point to at least `n` writable `u16` slots (the driver passes a
/// stack buffer sized for the read), valid for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn lw_mlx_rust_read_words(
    slave: u8,
    start_addr: u16,
    n: u16,
    out: *mut u16,
) -> i32 {
    with_model(|dev| {
        if dev.address() != slave {
            return 1; // NACK
        }
        dev.start();
        dev.write((start_addr >> 8) as u8);
        dev.write((start_addr & 0xFF) as u8);
        dev.start(); // repeated-start → read direction
        let out = unsafe { std::slice::from_raw_parts_mut(out, n as usize) };
        for w in out.iter_mut() {
            let hi = dev.read();
            let lo = dev.read();
            *w = ((hi as u16) << 8) | lo as u16;
        }
        dev.stop();
        0
    })
}

/// Write a single 16-bit word to `addr` (2 addr bytes + 2 data bytes, MSB
/// first), then STOP.
#[no_mangle]
pub extern "C" fn lw_mlx_rust_write_word(slave: u8, addr: u16, value: u16) -> i32 {
    with_model(|dev| {
        if dev.address() != slave {
            return 1; // NACK
        }
        dev.start();
        dev.write((addr >> 8) as u8);
        dev.write((addr & 0xFF) as u8);
        dev.write((value >> 8) as u8);
        dev.write((value & 0xFF) as u8);
        dev.stop();
        0
    })
}

unsafe extern "C" {
    fn lw_mlx_decode_scene(slave: u8, emissivity: f32, tr: f32, result_out: *mut f32) -> i32;
    fn lw_mlx_decode_ta_vdd(slave: u8, ta_out: *mut f32, vdd_out: *mut f32) -> i32;
}

fn install(model: Mlx90640) {
    MODEL.with(|m| *m.borrow_mut() = Some(model));
}

/// Decode the full 768-pixel field through the real driver. Ta = the decoded
/// ambient (the model fixes it to 25 °C), ε = 1, Tr = Ta — matching the
/// linearized calibration.
fn decode_field() -> Vec<f32> {
    let mut ta = 0.0f32;
    let mut vdd = 0.0f32;
    let perr = unsafe { lw_mlx_decode_ta_vdd(MLX90640_ADDR, &mut ta, &mut vdd) };
    assert_eq!(perr, 0, "ExtractParameters must succeed (got {perr})");
    let mut result = vec![-999.0f32; 768];
    let err = unsafe { lw_mlx_decode_scene(MLX90640_ADDR, 1.0, ta, result.as_mut_ptr()) };
    assert_eq!(err, 0, "decode path must succeed (got {err})");
    result
}

#[test]
fn real_driver_decodes_ta_25_vdd_3v3() {
    let _g = lock_decode();
    install(Mlx90640::with_default_scene(MLX90640_ADDR));
    let mut ta = 0.0f32;
    let mut vdd = 0.0f32;
    let err = unsafe { lw_mlx_decode_ta_vdd(MLX90640_ADDR, &mut ta, &mut vdd) };
    assert_eq!(err, 0);
    assert!(
        (ta - 25.0).abs() < 0.1,
        "real driver Ta should be ~25 °C, got {ta}"
    );
    assert!(
        (vdd - 3.3).abs() < 0.05,
        "real driver VDD should be ~3.3 V, got {vdd}"
    );
}

#[test]
fn round_trip_ambient_25_hotspot_60_localizes_and_matches() {
    let _g = lock_decode();
    // Known scene: ambient 25 °C, a 60 °C hotspot at pixel (row 12, col 16).
    let scene = ThermalScene::from_config(
        25.0, // ambient
        12,   // hot_row
        16,   // hot_col
        0,    // hot_radius (single pixel)
        60.0, // hot_target
        1.0,  // load
        0.0,  // tau (reach target immediately)
        0.0,  // cooling
        None, // no fault
        0.5,  // frame period
    );
    install(Mlx90640::new(MLX90640_ADDR, scene));

    let result = decode_field();

    let hot_px = 12 * 32 + 16; // 400
    let ambient = 25.0f32;
    let hot = 60.0f32;

    // 1) Every pixel decoded within tolerance of the injected scene.
    let mut max_err = 0.0f32;
    let mut worst = 0usize;
    for (i, &v) in result.iter().enumerate() {
        let want = if i == hot_px { hot } else { ambient };
        let e = (v - want).abs();
        if e > max_err {
            max_err = e;
            worst = i;
        }
    }
    println!(
        "ASSERT round-trip max_err = {max_err:.4} °C at px {worst} \
         (decoded {:.3}, want {:.3})",
        result[worst],
        if worst == hot_px { hot } else { ambient }
    );
    assert!(
        max_err < 1.0,
        "per-pixel decode must match scene within 1 °C; max_err={max_err} at px {worst}"
    );

    // 2) The hotspot pixel decodes to ~60 °C.
    println!(
        "ASSERT hotspot px{hot_px} decoded = {:.3} °C (want 60)",
        result[hot_px]
    );
    assert!(
        (result[hot_px] - hot).abs() < 1.0,
        "hotspot must decode to ~60 °C, got {}",
        result[hot_px]
    );

    // 3) The hotspot localizes to the correct pixel (global maximum).
    let argmax = result
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .map(|(i, _)| i)
        .unwrap();
    println!("ASSERT hotspot localized at px {argmax} (want {hot_px})");
    assert_eq!(
        argmax, hot_px,
        "hottest decoded pixel must be the injected hotspot location"
    );
}

#[test]
fn round_trip_warm_scene_tracks_multiple_levels() {
    let _g = lock_decode();
    // A warmer ambient with a stronger hotspot, to show the map is not pinned
    // to one calibration point.
    let scene = ThermalScene::from_config(40.0, 6, 8, 1, 95.0, 1.0, 0.0, 0.0, None, 0.5);
    install(Mlx90640::new(MLX90640_ADDR, scene));
    let result = decode_field();

    // Ambient pixel.
    assert!(
        (result[0] - 40.0).abs() < 1.5,
        "ambient should decode ~40 °C, got {}",
        result[0]
    );
    // Hotspot centre (row 6, col 8 = px 200).
    let hot_px = 6 * 32 + 8;
    assert!(
        (result[hot_px] - 95.0).abs() < 1.5,
        "hotspot should decode ~95 °C, got {}",
        result[hot_px]
    );
}
