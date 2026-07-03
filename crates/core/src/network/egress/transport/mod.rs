// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Egress transports. Each `send` runs on the worker thread and may block.

pub mod http;
pub mod mqtt;
pub mod tcp;

pub use http::HttpPoster;
pub use mqtt::MqttPublisher;
pub use tcp::TcpSink;

use std::sync::{Arc, Mutex};

/// A destination for encoded egress payloads. `send` runs on the transport
/// worker thread, so blocking network I/O is allowed here (never on the sim
/// thread).
pub trait EgressTransport: Send {
    fn send(&mut self, payload: &[u8]) -> anyhow::Result<()>;
}

/// In-memory transport for deterministic tests; records every payload.
pub struct MemoryTransport {
    sink: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl MemoryTransport {
    pub fn new() -> Self {
        Self {
            sink: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// A clone of the shared record, so a test can inspect received payloads.
    pub fn handle(&self) -> Arc<Mutex<Vec<Vec<u8>>>> {
        Arc::clone(&self.sink)
    }
}

impl Default for MemoryTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl EgressTransport for MemoryTransport {
    fn send(&mut self, payload: &[u8]) -> anyhow::Result<()> {
        self.sink.lock().unwrap().push(payload.to_vec());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_transport_records_payloads() {
        let mut t = MemoryTransport::new();
        let handle = t.handle();
        t.send(b"hello").unwrap();
        t.send(b"world").unwrap();
        let got = handle.lock().unwrap();
        assert_eq!(&*got, &[b"hello".to_vec(), b"world".to_vec()]);
    }
}
