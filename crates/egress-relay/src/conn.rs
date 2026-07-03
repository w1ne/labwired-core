use crate::build::build_transport;
use crate::hello::{parse_hello, Allowlist};
use std::io::{Read, Write};
use tungstenite::{Message, WebSocket};

/// Handle one browser connection: first Text frame is the hello (validated
/// against the fixed allowlist), every subsequent Binary frame is forwarded to
/// the backend transport. A rejected hello closes the socket.
pub fn serve_connection<S: Read + Write>(
    ws: &mut WebSocket<S>,
    allow: &Allowlist,
) -> anyhow::Result<()> {
    // 1. Hello.
    let hello = loop {
        match ws.read()? {
            Message::Text(t) => break parse_hello(&t)?,
            Message::Ping(_) | Message::Pong(_) => continue,
            Message::Close(_) => return Ok(()),
            _ => anyhow::bail!("expected hello text frame first"),
        }
    };
    if !allow.permits(&hello) {
        tracing::warn!(target = %hello.url, "rejected egress target (not allowlisted)");
        let _ = ws.close(None);
        anyhow::bail!("target not allowlisted");
    }
    let mut transport = build_transport(&hello)?;

    // 2. Forward payload frames.
    loop {
        match ws.read()? {
            Message::Binary(b) => {
                if let Err(e) = transport.send(&b) {
                    tracing::warn!("backend send failed: {e:?}");
                    break;
                }
            }
            Message::Text(_) => { /* ignore post-hello text */ }
            Message::Ping(_) | Message::Pong(_) => {}
            Message::Close(_) => break,
            Message::Frame(_) => {}
        }
    }
    Ok(())
}

#[cfg(all(test, feature = "net-tests"))]
mod tests {
    use super::*;
    use crate::hello::{AllowEntry, Allowlist};
    use std::io::Read;
    use std::net::TcpListener;

    #[test]
    fn forwards_ws_payload_to_allowed_tcp_backend() {
        // Fake customer backend.
        let backend = TcpListener::bind("127.0.0.1:0").unwrap();
        let backend_addr = backend.local_addr().unwrap().to_string();
        let backend_reader = std::thread::spawn(move || {
            let (mut s, _) = backend.accept().unwrap();
            let mut buf = [0u8; 5];
            s.read_exact(&mut buf).unwrap();
            buf
        });

        // Relay WS listener.
        let relay = TcpListener::bind("127.0.0.1:0").unwrap();
        let relay_addr = relay.local_addr().unwrap();
        let allow = Allowlist {
            entries: vec![AllowEntry {
                transport: "tcp".into(),
                url: backend_addr.clone(),
            }],
        };
        let relay_thread = std::thread::spawn(move || {
            let (stream, _) = relay.accept().unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            let _ = serve_connection(&mut ws, &allow);
        });

        // Browser side.
        let (mut ws, _) = tungstenite::connect(format!("ws://{relay_addr}/")).unwrap();
        ws.send(tungstenite::Message::Text(format!(
            r#"{{"transport":"tcp","url":"{backend_addr}","encoding":"raw"}}"#
        )))
        .unwrap();
        ws.send(tungstenite::Message::Binary(b"hello".to_vec()))
            .unwrap();
        ws.close(None).ok();

        assert_eq!(&backend_reader.join().unwrap(), b"hello");
        relay_thread.join().unwrap();
    }
}
