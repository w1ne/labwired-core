// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The scanning lidar's wire codec, gated against physical silicon.
//!
//! `tests/fixtures/ydlidar/ydlidar_230400_22s.bin` is 22.16 s of the RX line of
//! a real unit (see `PROVENANCE.md` beside it). Every frame in it is decoded and
//! then re-encoded through the shipped codec; the result must be byte-identical.
//!
//! This is the difference between a plausible model and a faithful one. Three
//! properties of the format are invisible to a decoder — the always-set check
//! bit in the angle fields, quarter-millimetre distances, and the `<< 2`
//! intensity packing — and each would produce bytes the device never emits.

use labwired_core::peripherals::components::ydlidar::{
    angle_to_raw, checksum, encode_frame, raw_to_angle, YdLidar, HEADER,
};
use labwired_core::peripherals::device::UartStreamDevice;

const CAPTURE: &[u8] = include_bytes!("fixtures/ydlidar/ydlidar_230400_22s.bin");

/// One frame lifted out of the capture, in decoded form.
struct Frame {
    offset: usize,
    ct: u8,
    fsa: u16,
    lsa: u16,
    cs: u16,
    samples: Vec<(u8, u16)>,
    bytes: Vec<u8>,
}

/// Walk the capture, keeping only frames whose checksum verifies.
///
/// Resynchronisation is deliberate: on a checksum failure advance ONE byte, not
/// a whole frame, so a corrupt length cannot make the walker skip good frames
/// and silently shrink the population the gate runs over.
fn decode_capture(buf: &[u8]) -> (Vec<Frame>, usize) {
    let mut frames = Vec::new();
    let mut rejected = 0usize;
    let mut i = 0usize;
    while i + 10 <= buf.len() {
        if buf[i..i + 2] != HEADER {
            i += 1;
            continue;
        }
        let ct = buf[i + 2];
        let lsn = buf[i + 3] as usize;
        let fsa = u16::from_le_bytes([buf[i + 4], buf[i + 5]]);
        let lsa = u16::from_le_bytes([buf[i + 6], buf[i + 7]]);
        let cs = u16::from_le_bytes([buf[i + 8], buf[i + 9]]);
        let n = 10 + 3 * lsn;
        if i + n > buf.len() {
            break;
        }
        let samples: Vec<(u8, u16)> = (0..lsn)
            .map(|k| {
                let b = i + 10 + 3 * k;
                (buf[b], u16::from_le_bytes([buf[b + 1], buf[b + 2]]))
            })
            .collect();
        if checksum(ct, lsn as u8, fsa, lsa, &samples) != cs {
            rejected += 1;
            i += 1;
            continue;
        }
        frames.push(Frame {
            offset: i,
            ct,
            fsa,
            lsa,
            cs,
            samples,
            bytes: buf[i..i + n].to_vec(),
        });
        i += n;
    }
    (frames, rejected)
}

#[test]
fn capture_is_the_population_the_provenance_claims() {
    let (frames, rejected) = decode_capture(CAPTURE);
    assert_eq!(CAPTURE.len(), 306_098, "capture file changed size");
    assert_eq!(frames.len(), 3_794, "frame count changed");
    assert_eq!(rejected, 0, "the real device emitted no bad checksums");
    let samples: usize = frames.iter().map(|f| f.samples.len()).sum();
    assert_eq!(samples, 89_331, "sample count changed");
    let revolutions = frames.iter().filter(|f| f.ct & 1 == 1).count();
    assert_eq!(revolutions, 223, "revolution count changed");
}

#[test]
fn encoder_reproduces_every_captured_frame_byte_for_byte() {
    let (frames, _) = decode_capture(CAPTURE);
    assert!(!frames.is_empty(), "fixture decoded to nothing");
    let mut mismatches = Vec::new();
    for f in &frames {
        let ours = encode_frame(f.ct, f.fsa, f.lsa, &f.samples);
        if ours != f.bytes {
            mismatches.push(f.offset);
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} of {} frames re-encoded differently; first at byte offset {:?}",
        mismatches.len(),
        frames.len(),
        mismatches.first()
    );
}

#[test]
fn checksum_matches_the_device_on_every_frame() {
    let (frames, _) = decode_capture(CAPTURE);
    for f in &frames {
        assert_eq!(
            checksum(f.ct, f.samples.len() as u8, f.fsa, f.lsa, &f.samples),
            f.cs,
            "checksum mismatch at offset {}",
            f.offset
        );
    }
}

#[test]
fn angle_check_bit_is_set_in_every_captured_field() {
    // The property an encoder gets wrong if it only ever read a decoder.
    let (frames, _) = decode_capture(CAPTURE);
    for f in &frames {
        assert_eq!(f.fsa & 1, 1, "FSA check bit clear at offset {}", f.offset);
        assert_eq!(f.lsa & 1, 1, "LSA check bit clear at offset {}", f.offset);
    }
}

#[test]
fn intensity_is_six_bit_and_distance_is_quarter_millimetre() {
    let (frames, _) = decode_capture(CAPTURE);
    let mut distance_low_bits = [0usize; 4];
    for f in &frames {
        for &(intensity, distance) in &f.samples {
            assert_eq!(
                intensity & 3,
                0,
                "intensity low bits set at offset {}",
                f.offset
            );
            distance_low_bits[(distance & 3) as usize] += 1;
        }
    }
    // If distances were whole millimetres only index 0 would ever be hit, and
    // dividing the word by 4 would be lossy in a way nothing else reveals.
    for (residue, &count) in distance_low_bits.iter().enumerate() {
        assert!(
            count > 0,
            "no sample had distance & 3 == {residue}; distances are not quarter-mm after all"
        );
    }
}

#[test]
fn our_angle_codec_round_trips_the_captured_fields() {
    let (frames, _) = decode_capture(CAPTURE);
    for f in &frames {
        for raw in [f.fsa, f.lsa] {
            assert_eq!(
                angle_to_raw(raw_to_angle(raw)),
                raw,
                "angle codec lost information on 0x{raw:04X}"
            );
        }
    }
}

// ── the model, not just the codec ───────────────────────────────────────────

/// Drain a device the way the UART tick path does: credit one 1 ms tick, then
/// spend the earned budget with zero-time polls.
fn drain_ticks(dev: &mut YdLidar, ticks: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for _ in 0..ticks {
        let budget = dev.max_bytes_per_tick().max(1);
        let mut credit = 1000u32;
        for _ in 0..budget {
            let Some(b) = dev.poll(credit) else { break };
            out.push(b);
            credit = 0;
        }
    }
    out
}

#[test]
fn model_emits_at_the_rate_the_scan_produces_not_the_tick_rate() {
    let mut dev = YdLidar::new();
    let bytes = drain_ticks(&mut dev, 1000); // one second of simulated time
                                             // 4031 samples/s x 3 bytes, plus 10 bytes of overhead on each of the
                                             // 161.2 data frames, plus 10 zero packets of 13 = 13 836 B/s. The unit
                                             // measured 13 806 B/s. The default one-byte-per-tick budget would cap
                                             // this at 1 000 and no frame would ever complete.
    assert!(
        (13_500..=14_200).contains(&bytes.len()),
        "expected ~13836 bytes in one second, got {}",
        bytes.len()
    );
    // And it must NOT saturate the line: the real device idles between frames.
    assert!(
        bytes.len() < 20_000,
        "stream saturated the 23040 B/s line at {} B/s; the head is being \
         driven by the wire instead of by time",
        bytes.len()
    );
}

#[test]
fn model_output_decodes_as_valid_frames_at_the_declared_scan_rate() {
    let mut dev = YdLidar::new();
    let bytes = drain_ticks(&mut dev, 1000);
    let (frames, rejected) = decode_capture(&bytes);
    assert_eq!(rejected, 0, "our own stream failed its own checksum");
    assert!(
        frames.len() > 100,
        "only {} frames in a second",
        frames.len()
    );
    let revolutions = frames.iter().filter(|f| f.ct & 1 == 1).count();
    assert!(
        (9..=11).contains(&revolutions),
        "expected ~10 revolutions in one second at the 10 Hz default, got {revolutions}"
    );
    // CT on a start packet carries the spin rate in tenths of a hertz.
    let start = frames
        .iter()
        .find(|f| f.ct & 1 == 1)
        .expect("no start packet");
    assert_eq!(start.ct >> 1, 100, "start packet should report 10.0 Hz");
    assert_eq!(start.samples.len(), 1, "zero packet carries one sample");
}

#[test]
fn revolution_boundary_flushes_a_short_frame_like_the_device_does() {
    // The capture holds 145 frames of LSN 24 and 11 of LSN 23 across 223
    // revolutions. A model that only ever emits full frames would drop those
    // samples and no assertion on frame count would notice.
    let mut dev = YdLidar::new();
    let bytes = drain_ticks(&mut dev, 3000);
    let (frames, _) = decode_capture(&bytes);
    let partial = frames
        .iter()
        .filter(|f| f.ct & 1 == 0 && f.samples.len() < 25)
        .count();
    assert!(
        partial > 0,
        "no short frame in 3 s; revolution boundaries are not flushing"
    );
    for f in &frames {
        assert!(
            f.samples.len() <= 25,
            "frame at {} carries {} samples",
            f.offset,
            f.samples.len()
        );
    }
}

#[test]
fn declared_room_is_what_firmware_decodes() {
    // A 4 m x 3 m room with the scanner centred: straight ahead (0 deg, +Y)
    // must read 1.5 m and broadside (90 deg, +X) must read 2.0 m, AFTER the
    // decoder applies the angle correction. Without the generator inverting
    // that correction these land ~8 deg off and the scan still looks credible.
    let mut dev = YdLidar::new();
    let bytes = drain_ticks(&mut dev, 400);
    let (frames, _) = decode_capture(&bytes);

    let mut hits: Vec<(f64, f64)> = Vec::new();
    for f in frames.iter().filter(|f| f.samples.len() > 1) {
        let first = raw_to_angle(f.fsa);
        let last = raw_to_angle(f.lsa);
        let span = (last - first).rem_euclid(360.0);
        let step = span / (f.samples.len() - 1) as f64;
        for (k, &(_, distance)) in f.samples.iter().enumerate() {
            let mm = distance as f64 / 4.0;
            if mm <= 0.0 {
                continue;
            }
            let correction =
                labwired_core::peripherals::components::ydlidar::angle_correction_deg(mm);
            hits.push(((first + step * k as f64 + correction).rem_euclid(360.0), mm));
        }
    }
    assert!(!hits.is_empty(), "no decodable samples");

    let nearest_to = |target: f64| -> f64 {
        hits.iter()
            .min_by(|a, b| {
                let da = ((a.0 - target + 180.0).rem_euclid(360.0) - 180.0).abs();
                let db = ((b.0 - target + 180.0).rem_euclid(360.0) - 180.0).abs();
                da.partial_cmp(&db).unwrap()
            })
            .map(|&(_, mm)| mm)
            .unwrap()
    };
    let ahead = nearest_to(0.0);
    let broadside = nearest_to(90.0);
    assert!(
        (ahead - 1500.0).abs() < 60.0,
        "bearing 0 deg should hit the 1.5 m wall, read {ahead:.0} mm"
    );
    assert!(
        (broadside - 2000.0).abs() < 60.0,
        "bearing 90 deg should hit the 2.0 m wall, read {broadside:.0} mm"
    );
}

#[test]
fn a_driven_target_appears_at_the_bearing_it_was_driven_to() {
    use labwired_core::sim_input::SimInput;
    let mut dev = YdLidar::new();
    dev.set_input("target_bearing", 200.0).unwrap();
    dev.set_input("target_range", 700.0).unwrap();
    dev.set_input("target_width", 20.0).unwrap();

    let bytes = drain_ticks(&mut dev, 400);
    let (frames, _) = decode_capture(&bytes);
    let mut at_target = 0usize;
    for f in frames.iter().filter(|f| f.samples.len() > 1) {
        let first = raw_to_angle(f.fsa);
        let last = raw_to_angle(f.lsa);
        let span = (last - first).rem_euclid(360.0);
        let step = span / (f.samples.len() - 1) as f64;
        for (k, &(_, distance)) in f.samples.iter().enumerate() {
            let mm = distance as f64 / 4.0;
            if mm <= 0.0 {
                continue;
            }
            let correction =
                labwired_core::peripherals::components::ydlidar::angle_correction_deg(mm);
            let bearing = (first + step * k as f64 + correction).rem_euclid(360.0);
            let delta = ((bearing - 200.0 + 180.0).rem_euclid(360.0) - 180.0).abs();
            if delta <= 8.0 {
                assert!(
                    (mm - 700.0).abs() < 1.0,
                    "inside the target arc but read {mm:.0} mm at {bearing:.1} deg"
                );
                at_target += 1;
            }
        }
    }
    assert!(
        at_target > 10,
        "target arc produced only {at_target} samples"
    );
}
