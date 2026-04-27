// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! GPIO observers for the `labwired run` CLI.
//!
//! - `TracingGpioObserver` — emits `tracing::info!(target: "gpio", ...)` on
//!   every pin transition. Always installed.
//! - `JsonGpioObserver` — streams one JSON-line record per transition to a
//!   file. Installed when `--gpio-trace <path>` is supplied.

use labwired_core::peripherals::esp32s3::gpio::GpioObserver;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::Mutex;

/// Default GPIO observer: emits a `tracing::info!` event for every transition.
#[derive(Debug, Default)]
pub struct TracingGpioObserver;

impl TracingGpioObserver {
    pub fn new() -> Self {
        Self
    }
}

impl GpioObserver for TracingGpioObserver {
    fn on_pin_change(&self, pin: u8, from: bool, to: bool, sim_cycle: u64) {
        tracing::info!(
            target: "gpio",
            "GPIO{}: {}->{}  (cycle={})",
            pin,
            from as u8,
            to as u8,
            sim_cycle,
        );
    }
}

/// Streams `{"sim_cycle":N, "pin":P, "from":B, "to":B}` JSON lines to a file.
pub struct JsonGpioObserver {
    writer: Mutex<BufWriter<File>>,
}

impl JsonGpioObserver {
    pub fn new(path: &std::path::Path) -> std::io::Result<Self> {
        let file = File::create(path)?;
        Ok(Self {
            writer: Mutex::new(BufWriter::new(file)),
        })
    }
}

impl std::fmt::Debug for JsonGpioObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JsonGpioObserver")
    }
}

impl GpioObserver for JsonGpioObserver {
    fn on_pin_change(&self, pin: u8, from: bool, to: bool, sim_cycle: u64) {
        if let Ok(mut w) = self.writer.lock() {
            let _ = writeln!(
                w,
                "{{\"sim_cycle\":{},\"pin\":{},\"from\":{},\"to\":{}}}",
                sim_cycle, pin, from, to,
            );
            let _ = w.flush();
        }
    }
}
