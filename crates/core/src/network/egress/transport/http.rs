// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Minimal HTTP/1.1 POST egress transport (no external HTTP crate). Opens a
//! fresh connection per `send` (Connection: close).

use crate::network::egress::transport::EgressTransport;
use std::io::{Read, Write};
use std::net::TcpStream;

pub struct HttpPoster {
    host: String,
    port: u16,
    path: String,
}

impl HttpPoster {
    /// `url` like `http://host:8080/ingest`. Only plain HTTP is supported.
    /// One POST per flushed payload (Connection: close); no batching.
    pub fn new(url: String) -> anyhow::Result<Self> {
        let rest = url
            .strip_prefix("http://")
            .ok_or_else(|| anyhow::anyhow!("only http:// URLs supported: {url}"))?;
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse()?),
            None => (authority.to_string(), 80u16),
        };
        Ok(Self {
            host,
            port,
            path: path.to_string(),
        })
    }
}

impl EgressTransport for HttpPoster {
    fn send(&mut self, payload: &[u8]) -> anyhow::Result<()> {
        let mut stream = TcpStream::connect((self.host.as_str(), self.port))?;
        let head = format!(
            "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            self.path,
            self.host,
            payload.len()
        );
        stream.write_all(head.as_bytes())?;
        stream.write_all(payload)?;
        stream.flush()?;
        let mut resp = Vec::new();
        stream.read_to_end(&mut resp)?; // drain so the server sees a clean close
        Ok(())
    }
}

#[cfg(all(test, feature = "net-tests"))]
mod tests {
    use super::*;
    use crate::network::egress::transport::EgressTransport;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    #[test]
    fn posts_to_path() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (sock, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(sock);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            // Drain the rest of the request (headers + body) so closing the
            // socket doesn't RST the client's in-flight write, then reply.
            let mut sock = reader.into_inner();
            sock.set_read_timeout(Some(std::time::Duration::from_millis(200)))
                .ok();
            let mut scratch = [0u8; 1024];
            let _ = sock.read(&mut scratch);
            let _ = sock
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            line
        });
        let mut poster =
            HttpPoster::new(format!("http://{}:{}/ingest", addr.ip(), addr.port())).unwrap();
        poster.send(b"body").unwrap();
        assert!(handle.join().unwrap().starts_with("POST /ingest"));
    }
}
