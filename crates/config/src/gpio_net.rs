// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Schema of the `gpio_net` environment interconnect: one electrical net that
//! joins GPIO pads of two or more world nodes (an interrupt line, a ready
//! line, a shared open-drain alert line).
//!
//! One typed struct is the single source of truth: the manifest validator
//! deserializes it to reject bad input before a world is built, and the world
//! builder deserializes the same struct to build the net.
//!
//! ```yaml
//! - type: gpio_net
//!   nodes: [stm, avr]            # every node that owns a member, once each
//!   config:
//!     name: alert                # optional label, shown in reports
//!     pull: up                   # none (default) | up | down
//!     latency_ns: 100            # default 100; zero is refused
//!     members:
//!       - { node: stm, peripheral: gpiob, pin: 4 }
//!       - { node: avr, peripheral: portd, pin: 2 }
//! ```

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Default wire delay between a member's pad edge and the other members
/// seeing it, ns.
pub const GPIO_NET_DEFAULT_LATENCY_NS: u64 = 100;

/// External pull on the net (a resistor to a rail). It is a weak level, like
/// a member chip's internal pull: it decides the wire when no member drives
/// it. When it disagrees with a member's internal pull, the net reports a pull
/// conflict and this pull wins (see `docs/howto/gpio-nets.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GpioNetPull {
    /// No pull: a net that nobody drives and no member pulls floats (reads 0
    /// and is flagged).
    #[default]
    None,
    Up,
    Down,
}

/// One pad on the net.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GpioNetMember {
    /// World node id.
    pub node: String,
    /// The node's GPIO peripheral id (`gpioa`, `portd`, `gpio`, ...).
    pub peripheral: String,
    /// Pin within that peripheral.
    pub pin: u8,
}

/// `config:` of a `gpio_net` interconnect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GpioNetConfig {
    #[serde(default)]
    pub name: Option<String>,
    pub members: Vec<GpioNetMember>,
    #[serde(default)]
    pub pull: GpioNetPull,
    #[serde(default = "default_latency_ns")]
    pub latency_ns: u64,
}

fn default_latency_ns() -> u64 {
    GPIO_NET_DEFAULT_LATENCY_NS
}

impl GpioNetConfig {
    /// Parse an interconnect's `config:` map.
    pub fn from_interconnect_config(config: &HashMap<String, serde_yaml::Value>) -> Result<Self> {
        let mut map = serde_yaml::Mapping::new();
        for (k, v) in config {
            map.insert(serde_yaml::Value::String(k.clone()), v.clone());
        }
        let parsed: Self =
            serde_yaml::from_value(serde_yaml::Value::Mapping(map)).context("gpio_net: config")?;
        parsed.validate()?;
        Ok(parsed)
    }

    /// Checks that need no node list.
    pub fn validate(&self) -> Result<()> {
        if self.latency_ns == 0 {
            anyhow::bail!(
                "gpio_net: latency_ns must be at least 1: a zero-delay net cannot be \
                 deterministic across machines"
            );
        }
        if self.members.len() < 2 {
            anyhow::bail!("gpio_net: a net needs at least two members");
        }
        let mut seen = std::collections::HashSet::new();
        for m in &self.members {
            if m.node.trim().is_empty() || m.peripheral.trim().is_empty() {
                anyhow::bail!("gpio_net: member node and peripheral must be non-empty");
            }
            if !seen.insert((m.node.as_str(), m.peripheral.as_str(), m.pin)) {
                anyhow::bail!(
                    "gpio_net: pad {}.{}.{} is listed twice",
                    m.node,
                    m.peripheral,
                    m.pin
                );
            }
        }
        if self
            .members
            .iter()
            .map(|m| m.node.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len()
            < 2
        {
            anyhow::bail!(
                "gpio_net: members must span at least two nodes (a net inside one chip is not a \
                 world net)"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> Result<GpioNetConfig> {
        let map: HashMap<String, serde_yaml::Value> = serde_yaml::from_str(yaml).unwrap();
        GpioNetConfig::from_interconnect_config(&map)
    }

    const TWO: &str =
        "members: [{ node: a, peripheral: gpioa, pin: 1 }, { node: b, peripheral: portd, pin: 2 }]";

    #[test]
    fn defaults_are_no_pull_and_100_ns() {
        let c = parse(TWO).unwrap();
        assert_eq!(c.pull, GpioNetPull::None);
        assert_eq!(c.latency_ns, 100);
    }

    #[test]
    fn zero_latency_is_refused() {
        let e = parse(&format!("{TWO}\nlatency_ns: 0")).unwrap_err();
        assert!(format!("{e:#}").contains("zero-delay"), "{e:#}");
    }

    #[test]
    fn a_net_needs_two_nodes_and_unique_pads() {
        assert!(parse("members: [{ node: a, peripheral: p, pin: 1 }]").is_err());
        assert!(parse(
            "members: [{ node: a, peripheral: p, pin: 1 }, { node: a, peripheral: p, pin: 2 }]"
        )
        .is_err());
        assert!(parse(
            "members: [{ node: a, peripheral: p, pin: 1 }, { node: b, peripheral: p, pin: 1 }, { node: b, peripheral: p, pin: 1 }]"
        )
        .is_err());
    }

    #[test]
    fn pull_and_unknown_keys() {
        assert_eq!(
            parse(&format!("{TWO}\npull: up")).unwrap().pull,
            GpioNetPull::Up
        );
        assert!(parse(&format!("{TWO}\npull: sideways")).is_err());
        assert!(parse(&format!("{TWO}\nlatency: 5")).is_err());
    }
}
