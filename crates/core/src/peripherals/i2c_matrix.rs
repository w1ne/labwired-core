// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Reaching an I²C slave through a **GPIO output matrix**.
//!
//! On the ESP32 family any I²C controller can drive any pad: the firmware
//! picks which one by writing the controller's SCL/SDA output signals into the
//! pads' `FUNCn_OUT_SEL_CFG`. A slave soldered to two pads therefore answers
//! whichever controller the firmware routes there — a fact about the board, not
//! about the controller a manifest happens to name.
//!
//! That matters when the two disagree. The canvas compiler names the
//! controller its pin table offers (`i2c0` on the ESP32-S3), while
//! `esp32-camera` opens its SCCB on I2C1 by default. On silicon the camera
//! answers I2C1 because the driver routes I2C1 to its pads; in the twin it only
//! answered the controller the manifest named.
//!
//! This module is the narrow fix. A slave attached with a pad route can be
//! given **twins** on the chip's other matrix-routed controllers
//! ([`crate::bus::SystemBus::attach_i2c_matrix_twins`]); a twin claims its
//! address only while the matrix sends its controller's SCL and SDA to the
//! route's pads. The named controller keeps reaching the slave exactly as
//! before, so nothing that works today changes. Which parts get twins is the
//! part's decision (its kit asks), because a twin must be a second handle on
//! the SAME device state, which only the part knows how to make.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::peripherals::i2c::I2cDevice;

/// A GPIO output matrix: which peripheral output signal drives each pad now.
pub trait PadOutMatrix: std::fmt::Debug + Send + Sync {
    /// The output-signal index routed to `pad`, or `None` for a pad the matrix
    /// does not have.
    fn out_signal(&self, pad: u8) -> Option<u32>;
}

/// How a matrix-routed I²C controller reaches pads: the live matrix and the
/// controller's own SCL/SDA output-signal indices.
#[derive(Debug, Clone)]
pub struct I2cMatrixReach {
    pub matrix: Arc<dyn PadOutMatrix>,
    pub scl_signal: u32,
    pub sda_signal: u32,
}

impl I2cMatrixReach {
    /// `true` while the matrix sends this controller's SCL to `scl_pad` and its
    /// SDA to `sda_pad`.
    pub fn routes_to(&self, scl_pad: u8, sda_pad: u8) -> bool {
        self.matrix.out_signal(scl_pad) == Some(self.scl_signal)
            && self.matrix.out_signal(sda_pad) == Some(self.sda_signal)
    }
}

/// The `(scl, sda)` pad numbers of a manifest route (`{scl: "GPIO5", sda:
/// "GPIO4"}`), or `None` when either is missing or not a `GPIOn` pad.
pub fn route_pads(route: &BTreeMap<String, String>) -> Option<(u8, u8)> {
    let pad = |key: &str| -> Option<u8> {
        let name = route.get(key)?;
        name.strip_prefix("GPIO")
            .or_else(|| name.strip_prefix("gpio"))?
            .parse()
            .ok()
    };
    Some((pad("scl")?, pad("sda")?))
}

/// A slave twin that a controller reaches only through the output matrix.
///
/// Wire operations forward to the twin; the address is claimed only while the
/// controller's SCL/SDA are routed to the twin's pads. Inspection forwards too
/// (`wrapped_device`), but a twin is a second handle on a device the named
/// controller already exposes, so whatever it shows the primary shows as well.
pub struct MatrixRoutedI2c {
    inner: Box<dyn I2cDevice>,
    reach: I2cMatrixReach,
    scl_pad: u8,
    sda_pad: u8,
}

impl MatrixRoutedI2c {
    pub fn new(inner: Box<dyn I2cDevice>, reach: I2cMatrixReach, scl_pad: u8, sda_pad: u8) -> Self {
        Self {
            inner,
            reach,
            scl_pad,
            sda_pad,
        }
    }

    fn routed(&self) -> bool {
        self.reach.routes_to(self.scl_pad, self.sda_pad)
    }
}

impl I2cDevice for MatrixRoutedI2c {
    fn transaction_generation(&self) -> Option<u64> {
        self.inner.transaction_generation()
    }
    fn address(&self) -> u8 {
        self.inner.address()
    }
    fn read(&mut self) -> u8 {
        self.inner.read()
    }
    fn write(&mut self, data: u8) {
        self.inner.write(data);
    }
    fn start(&mut self) {
        self.inner.start();
    }
    fn stop(&mut self) {
        self.inner.stop();
    }
    fn claims_address(&self, addr: u8) -> bool {
        self.routed() && self.inner.claims_address(addr)
    }
    fn select_address(&mut self, addr: u8) {
        self.inner.select_address(addr);
    }
    fn wrapped_device(&self) -> Option<&dyn I2cDevice> {
        Some(self.inner.as_ref())
    }
    fn wrapped_device_mut(&mut self) -> Option<&mut dyn I2cDevice> {
        Some(self.inner.as_mut())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[derive(Debug)]
    struct Matrix([AtomicU32; 8]);

    impl PadOutMatrix for Matrix {
        fn out_signal(&self, pad: u8) -> Option<u32> {
            self.0.get(pad as usize).map(|s| s.load(Ordering::Relaxed))
        }
    }

    struct Probe(u8);

    impl I2cDevice for Probe {
        fn address(&self) -> u8 {
            self.0
        }
        fn read(&mut self) -> u8 {
            0xA5
        }
        fn write(&mut self, _data: u8) {}
    }

    #[test]
    fn a_twin_answers_only_while_its_controller_is_routed_to_the_pads() {
        let matrix = Arc::new(Matrix(Default::default()));
        let reach = I2cMatrixReach {
            matrix: matrix.clone(),
            scl_signal: 91,
            sda_signal: 92,
        };
        let twin = MatrixRoutedI2c::new(Box::new(Probe(0x3C)), reach, 5, 4);
        assert!(!twin.claims_address(0x3C), "unrouted: silent");
        matrix.0[5].store(91, Ordering::Relaxed);
        assert!(!twin.claims_address(0x3C), "SCL alone is not enough");
        matrix.0[4].store(92, Ordering::Relaxed);
        assert!(twin.claims_address(0x3C), "both pads routed: answers");
        assert!(!twin.claims_address(0x21), "still only its own address");
        matrix.0[4].store(90, Ordering::Relaxed);
        assert!(
            !twin.claims_address(0x3C),
            "SDA moved to another controller"
        );
    }

    #[test]
    fn route_pads_reads_gpio_names() {
        let route: BTreeMap<String, String> = [("scl", "GPIO5"), ("sda", "GPIO4")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        assert_eq!(route_pads(&route), Some((5, 4)));
        let bad: BTreeMap<String, String> = [("scl".to_string(), "PB6".to_string())].into();
        assert_eq!(route_pads(&bad), None);
    }
}
