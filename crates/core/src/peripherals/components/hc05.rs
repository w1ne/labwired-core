// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! HC-05 Bluetooth SPP module — thin AT command shell over UART.
//!
//! Responds `OK\r\n` to `AT` / `AT\r\n` and common `AT+…` lines. Not a full
//! RF stack: pairing and wireless data are not modelled.

use crate::peripherals::uart::UartStreamDevice;
use std::any::Any;
use std::collections::VecDeque;

pub struct Hc05 {
    rx_line: String,
    out_queue: VecDeque<u8>,
    component_id: Option<String>,
}

impl Default for Hc05 {
    fn default() -> Self {
        Self::new()
    }
}

impl Hc05 {
    pub fn new() -> Self {
        Self {
            rx_line: String::new(),
            out_queue: VecDeque::new(),
            component_id: None,
        }
    }

    fn enqueue_str(&mut self, s: &str) {
        for b in s.bytes() {
            self.out_queue.push_back(b);
        }
    }

    fn handle_line(&mut self, line: &str) {
        let t = line.trim().trim_end_matches('\r');
        if t.is_empty() {
            return;
        }
        let upper = t.to_ascii_uppercase();
        if upper == "AT" || upper.starts_with("AT+") || upper.starts_with("AT ") {
            if upper == "AT+VERSION?" || upper.starts_with("AT+VERSION") {
                self.enqueue_str("+VERSION:labwired-hc05-sim\r\nOK\r\n");
            } else if upper == "AT+NAME?" {
                self.enqueue_str("+NAME:HC-05\r\nOK\r\n");
            } else {
                self.enqueue_str("OK\r\n");
            }
        } else {
            // Data mode: echo nothing; thin shell only handles AT.
            self.enqueue_str("ERROR\r\n");
        }
    }
}

impl UartStreamDevice for Hc05 {
    fn poll(&mut self, _elapsed_us: u32) -> Option<u8> {
        self.out_queue.pop_front()
    }

    fn on_tx_byte(&mut self, byte: u8) {
        if byte == b'\n' {
            let line = std::mem::take(&mut self.rx_line);
            self.handle_line(&line);
        } else if byte != b'\r' {
            if self.rx_line.len() < 128 {
                self.rx_line.push(byte as char);
            }
        } else if !self.rx_line.is_empty() {
            // CR without LF: some hosts send AT\r only
            let line = std::mem::take(&mut self.rx_line);
            self.handle_line(&line);
        }
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

use crate::peripherals::kit::{AttachCtx, Category, KitMetadata, PeripheralKit, Transport};

pub struct Hc05Kit;
pub static HC05_KIT: Hc05Kit = Hc05Kit;

static HC05_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "hc-05",
    label: "HC-05 Bluetooth",
    summary: "Bluetooth SPP module with AT command shell over UART.",
    detail: "Thin AT echo model (OK to AT/AT+…). No RF pairing or wireless data path.",
    transport: Transport::Uart,
    category: Category::Uart,
    config_keys: &[],
    labs: &[],
};

impl PeripheralKit for Hc05Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &HC05_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let mut dev = Hc05::new();
        dev.component_id = Some(ctx.device_id().to_string());
        let uart = ctx.uart()?;
        uart.attach_stream(Box::new(dev));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_returns_ok() {
        let mut dev = Hc05::new();
        for b in b"AT\r\n" {
            dev.on_tx_byte(*b);
        }
        let mut out = String::new();
        while let Some(b) = dev.poll(0) {
            out.push(b as char);
        }
        assert_eq!(out, "OK\r\n");
    }

    #[test]
    fn at_version() {
        let mut dev = Hc05::new();
        for b in b"AT+VERSION?\r\n" {
            dev.on_tx_byte(*b);
        }
        let mut out = String::new();
        while let Some(b) = dev.poll(0) {
            out.push(b as char);
        }
        assert!(out.contains("VERSION"));
        assert!(out.contains("OK"));
    }
}
