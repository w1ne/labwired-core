// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::SimResult;
use std::io::{self, Write};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UartRegisterLayout {
    #[default]
    Stm32F1,
    Stm32V2,
}

impl FromStr for UartRegisterLayout {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let v = value.trim().to_ascii_lowercase();
        match v.as_str() {
            "stm32f1" | "f1" | "legacy" => Ok(Self::Stm32F1),
            "stm32v2" | "v2" | "modern" | "stm32-modern" | "h5" | "stm32h5" => Ok(Self::Stm32V2),
            _ => Err(format!(
                "unsupported UART register layout '{}'; supported: stm32f1, stm32v2",
                value
            )),
        }
    }
}

/// Minimal UART mock with selectable register layout.
#[derive(Debug, Default, serde::Serialize)]
pub struct Uart {
    layout: UartRegisterLayout,
    #[serde(skip)]
    sink: Option<Arc<Mutex<Vec<u8>>>>,
    echo_stdout: bool,
}

impl Uart {
    pub fn new() -> Self {
        Self::new_with_layout(UartRegisterLayout::Stm32F1)
    }

    pub fn new_with_layout(layout: UartRegisterLayout) -> Self {
        Self {
            layout,
            sink: None,
            echo_stdout: true,
        }
    }

    fn status_offset(&self) -> u64 {
        match self.layout {
            UartRegisterLayout::Stm32F1 => 0x00,
            UartRegisterLayout::Stm32V2 => 0x1C, // ISR
        }
    }

    fn tx_offset(&self) -> u64 {
        match self.layout {
            UartRegisterLayout::Stm32F1 => 0x04, // DR
            UartRegisterLayout::Stm32V2 => 0x28, // TDR
        }
    }

    fn rx_offset(&self) -> u64 {
        match self.layout {
            UartRegisterLayout::Stm32F1 => 0x04, // DR
            UartRegisterLayout::Stm32V2 => 0x24, // RDR
        }
    }

    fn status_ready_value(&self) -> u8 {
        0xC0 // TX-ready + TC-ready in low byte for both layouts.
    }

    fn push_tx(&mut self, value: u8) {
        if let Some(sink) = &self.sink {
            if let Ok(mut guard) = sink.lock() {
                guard.push(value);
            }
        }

        if self.echo_stdout {
            #[allow(unused_must_use)]
            {
                print!("{}", value as char);
                io::stdout().flush();
            }
        }
    }

    pub fn set_sink(&mut self, sink: Option<Arc<Mutex<Vec<u8>>>>, echo_stdout: bool) {
        self.sink = sink;
        self.echo_stdout = echo_stdout;
    }
}

impl crate::Peripheral for Uart {
    fn read(&self, offset: u64) -> SimResult<u8> {
        if offset == self.status_offset() {
            return Ok(self.status_ready_value());
        }
        if offset == self.rx_offset() {
            return Ok(0x00);
        }
        Ok(0)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let is_legacy_tx_alias =
            matches!(self.layout, UartRegisterLayout::Stm32F1) && offset == 0x00;
        if offset == self.tx_offset() || is_legacy_tx_alias {
            self.push_tx(value);
        }
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        if offset == self.status_offset() {
            return Some(self.status_ready_value());
        }
        if offset == self.rx_offset() {
            return Some(0x00);
        }
        Some(0)
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::{Uart, UartRegisterLayout};
    use crate::Peripheral;
    use std::sync::{Arc, Mutex};

    #[test]
    fn test_uart_f1_transmit_offsets() {
        let mut uart = Uart::new_with_layout(UartRegisterLayout::Stm32F1);
        let sink = Arc::new(Mutex::new(Vec::new()));
        uart.set_sink(Some(sink.clone()), false);

        // DR offset
        uart.write(0x04, b'A').unwrap();
        // Legacy alias for compatibility in existing fixtures
        uart.write(0x00, b'B').unwrap();

        let data = sink.lock().unwrap().clone();
        assert_eq!(data, vec![b'A', b'B']);
    }

    #[test]
    fn test_uart_v2_transmit_uses_tdr_only() {
        let mut uart = Uart::new_with_layout(UartRegisterLayout::Stm32V2);
        let sink = Arc::new(Mutex::new(Vec::new()));
        uart.set_sink(Some(sink.clone()), false);

        // Wrong offset for v2 should not transmit.
        uart.write(0x04, b'X').unwrap();
        // TDR offset
        uart.write(0x28, b'Y').unwrap();

        let data = sink.lock().unwrap().clone();
        assert_eq!(data, vec![b'Y']);
        assert_eq!(uart.read(0x1C).unwrap(), 0xC0); // ISR ready flags
    }
}
