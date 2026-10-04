// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

use labwired_core::peripherals::components::mlx90640::{Mlx90640, ThermalScene, MLX90640_ADDR};
use labwired_core::peripherals::i2c::I2cDevice;

fn words(dev: &mut Mlx90640, address: u16, count: usize) -> Vec<u16> {
    dev.start();
    dev.write((address >> 8) as u8);
    dev.write(address as u8);
    dev.start();
    let result = (0..count)
        .map(|_| u16::from_be_bytes([dev.read(), dev.read()]))
        .collect();
    dev.stop();
    result
}

#[test]
fn odd_skipped_conversions_with_prior_partial_period_match_fine_ticks() {
    let scene = ThermalScene::from_config(25.0, 12, 16, 2, 75.0, 1.0, 0.4, 0.7, Some(1.8), 0.5);
    let mut coarse = Mlx90640::new(MLX90640_ADDR, scene.clone());
    let mut fine = Mlx90640::new(MLX90640_ADDR, scene);
    coarse.advance_time_us(125_000);
    fine.advance_time_us(125_000);
    // Seven conversions: skip five, retain the last two. Carry the prior
    // 125 ms fraction, including a cooling-fault boundary during the skip.
    coarse.advance_time_us(3_625_000);
    for _ in 0..3625 {
        fine.advance_time_us(1000);
    }
    assert!((coarse.scene().elapsed_s() - 3.75).abs() < 1e-10);
    assert!((coarse.scene().hotspot_c() - fine.scene().hotspot_c()).abs() < 1e-10);
    assert_eq!(words(&mut coarse, 0x8000, 1), words(&mut fine, 0x8000, 1));
    assert_eq!(
        words(&mut coarse, 0x0400, 832),
        words(&mut fine, 0x0400, 832)
    );
}

#[test]
fn very_long_idle_advance_keeps_sampling_bounded_and_temperature_finite() {
    let mut dev = Mlx90640::with_default_scene(MLX90640_ADDR);
    dev.advance_time_us(499_999);
    dev.advance_time_us(u64::MAX);
    assert!(dev.scene().hotspot_c().is_finite());
    assert_eq!(dev.scene().hotspot_c(), 60.0);
    assert_ne!(words(&mut dev, 0x8000, 1)[0] & 8, 0);
}
