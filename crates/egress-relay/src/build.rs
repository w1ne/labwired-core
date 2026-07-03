use crate::hello::Hello;
use anyhow::Context;
use labwired_core::network::egress::transport::{
    EgressTransport, HttpPoster, MqttPublisher, TcpSink,
};

/// Map a validated hello to a native (blocking) egress transport. Transports
/// connect lazily on first send, so this never touches the network.
pub fn build_transport(h: &Hello) -> anyhow::Result<Box<dyn EgressTransport>> {
    match h.transport.as_str() {
        "tcp" => Ok(Box::new(TcpSink::new(h.url.clone()))),
        "mqtt" => {
            let (host, port) = parse_mqtt_url(&h.url)?;
            let topic = h.topic.clone().context("mqtt hello needs 'topic'")?;
            Ok(Box::new(MqttPublisher::lazy(host, port, topic)))
        }
        "http" => Ok(Box::new(HttpPoster::new(h.url.clone())?)),
        other => anyhow::bail!("unknown transport '{other}'"),
    }
}

/// `mqtt://host:port` -> (host, port). Duplicated from world.rs deliberately —
/// the relay must not depend on the World wiring.
fn parse_mqtt_url(url: &str) -> anyhow::Result<(String, u16)> {
    let rest = url.strip_prefix("mqtt://").unwrap_or(url);
    let (host, port) = rest
        .rsplit_once(':')
        .context("mqtt url must be mqtt://host:port")?;
    Ok((host.to_string(), port.parse()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hello::parse_hello;

    #[test]
    fn builds_tcp_transport() {
        let h = parse_hello(r#"{"transport":"tcp","url":"127.0.0.1:9000","encoding":"raw"}"#).unwrap();
        assert!(build_transport(&h).is_ok());
    }

    #[test]
    fn mqtt_requires_topic() {
        let h = parse_hello(r#"{"transport":"mqtt","url":"mqtt://h:1883","encoding":"raw"}"#).unwrap();
        assert!(build_transport(&h).is_err());
    }

    #[test]
    fn rejects_unknown_transport() {
        let h = parse_hello(r#"{"transport":"carrier-pigeon","url":"x","encoding":"raw"}"#).unwrap();
        assert!(build_transport(&h).is_err());
    }
}
