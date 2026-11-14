// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use labwired_core::peripherals::noise::ChannelNoise;

#[test]
fn same_seed_replays_identically() {
    let mut a = ChannelNoise::new(0, "imu", "ax", 0.05, 0.0, None);
    let mut b = ChannelNoise::new(0, "imu", "ax", 0.05, 0.0, None);
    for _ in 0..1000 {
        assert_eq!(a.sample(1.0, None), b.sample(1.0, None));
    }
}

#[test]
fn different_channels_diverge() {
    let mut a = ChannelNoise::new(0, "imu", "ax", 0.05, 0.0, None);
    let mut b = ChannelNoise::new(0, "imu", "ay", 0.05, 0.0, None);
    let seq_a: Vec<f64> = (0..16).map(|_| a.sample(1.0, None)).collect();
    let seq_b: Vec<f64> = (0..16).map(|_| b.sample(1.0, None)).collect();
    assert_ne!(seq_a, seq_b);
}

#[test]
fn zero_sigma_is_bias_only() {
    let mut n = ChannelNoise::new(0, "t", "temperature", 0.0, 1.5, None);
    assert_eq!(n.sample(20.0, None), 21.5);
}

#[test]
fn gaussian_has_requested_sigma() {
    let mut n = ChannelNoise::new(7, "imu", "ax", 0.1, 0.0, None);
    let n_samples = 20_000;
    let samples: Vec<f64> = (0..n_samples).map(|_| n.sample(0.0, None)).collect();
    let mean = samples.iter().sum::<f64>() / n_samples as f64;
    let var = samples.iter().map(|s| (s - mean).powi(2)).sum::<f64>() / n_samples as f64;
    assert!(mean.abs() < 0.01, "mean {mean} drifted");
    let sigma = var.sqrt();
    assert!((sigma - 0.1).abs() < 0.01, "sigma {sigma} off");
}

#[test]
fn thermal_lag_smooths_step() {
    let mut n = ChannelNoise::new(0, "t", "temperature", 0.0, 0.0, Some(1.0));
    // Establish a 20 °C baseline, then step to 100 °C: the lag filter must
    // approach the step gradually (tau = 1 s), not jump.
    let base = n.sample(20.0, Some(0));
    assert_eq!(base, 20.0);
    let after_step = n.sample(100.0, Some(100_000)); // +0.1 s
    assert!(
        after_step > 20.0,
        "must move toward the step, got {after_step}"
    );
    assert!(after_step < 100.0, "must not reach the step within 0.1 tau");
    let later = n.sample(100.0, Some(2_100_000)); // +2 s more
    assert!(later > after_step, "must keep converging");
    assert!(later < 100.0, "must still be converging, got {later}");
}
