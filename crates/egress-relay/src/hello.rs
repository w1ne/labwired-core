use serde::Deserialize;

/// The first frame a browser sends: what backend it wants to reach.
#[derive(Debug, Clone, Deserialize)]
pub struct Hello {
    pub transport: String,
    pub url: String,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default = "default_encoding")]
    pub encoding: String,
}

fn default_encoding() -> String {
    "raw".to_string()
}

pub fn parse_hello(text: &str) -> anyhow::Result<Hello> {
    Ok(serde_json::from_str(text)?)
}

/// One permitted (transport, url) target. Fixed at deploy time; never derived
/// from caller input.
#[derive(Debug, Clone)]
pub struct AllowEntry {
    pub transport: String,
    pub url: String,
}

#[derive(Debug, Clone, Default)]
pub struct Allowlist {
    pub entries: Vec<AllowEntry>,
}

impl Allowlist {
    pub fn permits(&self, h: &Hello) -> bool {
        self.entries
            .iter()
            .any(|e| e.transport == h.transport && e.url == h.url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_valid_mqtt_hello() {
        let h = parse_hello(r#"{"v":1,"transport":"mqtt","url":"mqtt://demo.internal:1883","topic":"demo/temp","encoding":"raw"}"#).unwrap();
        assert_eq!(h.transport, "mqtt");
        assert_eq!(h.topic.as_deref(), Some("demo/temp"));
    }

    #[test]
    fn allowlist_permits_only_listed_target() {
        let allow = Allowlist { entries: vec![AllowEntry {
            transport: "mqtt".into(), url: "mqtt://demo.internal:1883".into(),
        }] };
        let ok = parse_hello(r#"{"transport":"mqtt","url":"mqtt://demo.internal:1883","topic":"t","encoding":"raw"}"#).unwrap();
        let bad = parse_hello(r#"{"transport":"mqtt","url":"mqtt://evil.example:1883","topic":"t","encoding":"raw"}"#).unwrap();
        assert!(allow.permits(&ok));
        assert!(!allow.permits(&bad));
    }
}
