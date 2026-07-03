// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Raw-TCP egress transport. Connects lazily on the first `send` so manifest
//! construction never blocks on the network.

use crate::network::egress::transport::EgressTransport;
use std::io::Write;
use std::net::TcpStream;

pub struct TcpSink {
    addr: String,
    stream: Option<TcpStream>,
}

impl TcpSink {
    /// Lazy: stores the address; connects on the first `send`.
    pub fn new(addr: String) -> Self {
        Self { addr, stream: None }
    }

    /// Eager: connects immediately (used by the net-tests).
    pub fn connect(addr: &str) -> anyhow::Result<Self> {
        let stream = TcpStream::connect(addr)?;
        Ok(Self {
            addr: addr.to_string(),
            stream: Some(stream),
        })
    }

    fn stream(&mut self) -> anyhow::Result<&mut TcpStream> {
        if self.stream.is_none() {
            self.stream = Some(TcpStream::connect(&self.addr)?);
        }
        Ok(self.stream.as_mut().unwrap())
    }
}

impl EgressTransport for TcpSink {
    fn send(&mut self, payload: &[u8]) -> anyhow::Result<()> {
        let stream = self.stream()?;
        stream.write_all(payload)?;
        stream.flush()?;
        Ok(())
    }
}

#[cfg(all(test, feature = "net-tests"))]
mod tests {
    use super::*;
    use crate::network::egress::transport::EgressTransport;
    use std::io::Read;
    use std::net::TcpListener;

    #[test]
    fn tcp_sink_writes_payload_to_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let handle = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 5];
            sock.read_exact(&mut buf).unwrap();
            buf
        });
        let mut sink = TcpSink::connect(&addr).unwrap();
        sink.send(b"hello").unwrap();
        assert_eq!(&handle.join().unwrap(), b"hello");
    }
}
