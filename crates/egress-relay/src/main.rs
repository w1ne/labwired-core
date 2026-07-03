use labwired_egress_relay::hello::{AllowEntry, Allowlist};
use labwired_egress_relay::conn::serve_connection;
use std::net::TcpListener;

/// Build the fixed allowlist from environment (injectable `get` for tests).
/// MVP: a single demo target; extend the vec to allowlist more fixed backends.
fn allow_from_env(get: impl Fn(&str) -> Option<String>) -> Allowlist {
    Allowlist {
        entries: vec![AllowEntry {
            transport: get("RELAY_ALLOW_TRANSPORT").unwrap_or_else(|| "mqtt".into()),
            url: get("RELAY_ALLOW_URL").unwrap_or_else(|| "mqtt://127.0.0.1:1883".into()),
        }],
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber_init();
    // MVP: fixed allowlist from env. Point at the demo broker only.
    let allow = allow_from_env(|k| std::env::var(k).ok());
    let bind = std::env::var("RELAY_BIND").unwrap_or_else(|_| "127.0.0.1:8090".into());
    let listener = TcpListener::bind(&bind)?;
    tracing::info!(%bind, "egress relay listening");

    for stream in listener.incoming() {
        let stream = match stream { Ok(s) => s, Err(_) => continue };
        let allow = allow.clone();
        std::thread::spawn(move || {
            let mut ws = match tungstenite::accept(stream) { Ok(w) => w, Err(_) => return };
            if let Err(e) = serve_connection(&mut ws, &allow) {
                tracing::debug!("connection ended: {e:?}");
            }
        });
    }
    Ok(())
}

fn tracing_subscriber_init() {
    // Best-effort; ignore if a global subscriber already exists.
    let _ = tracing::subscriber::set_global_default(
        tracing::subscriber::NoSubscriber::default(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn env_builds_single_allowlist_entry() {
        let env: HashMap<&str, &str> = [
            ("RELAY_ALLOW_TRANSPORT", "mqtt"),
            ("RELAY_ALLOW_URL", "mqtt://demo.internal:1883"),
        ].into_iter().collect();
        let allow = allow_from_env(|k| env.get(k).map(|s| s.to_string()));
        assert_eq!(allow.entries.len(), 1);
        assert_eq!(allow.entries[0].transport, "mqtt");
        assert_eq!(allow.entries[0].url, "mqtt://demo.internal:1883");
    }

    #[test]
    fn env_falls_back_to_local_defaults() {
        let allow = allow_from_env(|_| None);
        assert_eq!(allow.entries.len(), 1);
        assert!(allow.entries[0].url.starts_with("mqtt://127.0.0.1"));
    }
}
