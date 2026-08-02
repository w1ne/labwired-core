// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SIM800L GSM module — thin AT command shell over UART (no cellular RF).

use crate::peripherals::uart::UartStreamDevice;
use std::any::Any;
use std::collections::VecDeque;

pub struct Sim800l {
    rx_line: String,
    out_queue: VecDeque<u8>,
    component_id: Option<String>,
}

impl Default for Sim800l {
    fn default() -> Self {
        Self::new()
    }
}

impl Sim800l {
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
            if upper == "AT+CGMI" {
                self.enqueue_str("SIMCOM_Ltd\r\nOK\r\n");
            } else if upper == "AT+CGMM" {
                self.enqueue_str("SIMCOM_SIM800L\r\nOK\r\n");
            } else if upper == "ATI" || upper == "AT+GMR" {
                self.enqueue_str("SIM800L R14.18 LabWired\r\nOK\r\n");
            } else if upper.starts_with("AT+CSQ") {
                self.enqueue_str("+CSQ: 20,0\r\nOK\r\n");
            } else if upper.starts_with("AT+CREG") {
                self.enqueue_str("+CREG: 0,1\r\nOK\r\n");
            } else {
                self.enqueue_str("OK\r\n");
            }
        } else {
            self.enqueue_str("ERROR\r\n");
        }
    }
}

impl UartStreamDevice for Sim800l {
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

pub struct Sim800lKit;
pub static SIM800L_KIT: Sim800lKit = Sim800lKit;

static SIM800L_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "sim800l",
    label: "SIM800L GSM",
    summary: "SIM800L UART AT command shell (no cellular RF).",
    detail: "Responds OK to AT/AT+…; CGMI/CGMM/CSQ/CREG stubs for init sequences. \
             SMS/GPRS air interface is not simulated.",
    transport: Transport::Uart,
    category: Category::Uart,
    config_keys: &[],
    labs: &[],
};

impl PeripheralKit for Sim800lKit {
    fn metadata(&self) -> &'static KitMetadata {
        &SIM800L_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let mut dev = Sim800l::new();
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
        let mut d = Sim800l::new();
        for b in b"AT\r\n" {
            d.on_tx_byte(*b);
        }
        let mut out = String::new();
        while let Some(b) = d.poll(0) {
            out.push(b as char);
        }
        assert!(out.contains("OK"), "{out}");
    }
}
