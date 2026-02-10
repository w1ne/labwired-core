// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::SimResult;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

/// Simple UART mock.
/// Writes to Data Register (offset 0x0) correspond to stdout writes.
#[derive(Debug, Default, serde::Serialize)]
pub struct Uart {
    #[serde(skip)]
    sink: Option<Arc<Mutex<Vec<u8>>>>,
    echo_stdout: bool,
}

impl Uart {
    pub fn new() -> Self {
        Self {
            sink: None,
            echo_stdout: true,
        }
    }

    pub fn set_sink(&mut self, sink: Option<Arc<Mutex<Vec<u8>>>>, echo_stdout: bool) {
        self.sink = sink;
        self.echo_stdout = echo_stdout;
    }
}

impl crate::Peripheral for Uart {
    fn read(&self, offset: u64) -> SimResult<u8> {
        match offset {
            0x00 => Ok(0xC0), // SR: TXE=1, TC=1 (Ready)
            0x04 => Ok(0x00), // DR: Always return 0 for reads
            _ => Ok(0),
        }
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        // STM32 USART DR is at offset 0x04
        if offset == 0x04 || offset == 0x00 {
            if let Some(sink) = &self.sink {
                tracing::info!("UART WRITE: {:#02x}", value);
                if let Ok(mut guard) = sink.lock() {
                    guard.push(value);
                }
            } else {
                tracing::info!("UART WRITE (NO SINK): {:#02x}", value);
            }

            if self.echo_stdout {
                // Write to Data Register -> Stdout
                #[allow(unused_must_use)]
                {
                    print!("{}", value as char);
                    io::stdout().flush();
                }
            }
        }
        Ok(())
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
