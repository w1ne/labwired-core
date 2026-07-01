// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! IFM-inspired multi-port IO-Link master station — a product/profile model.
//!
//! This is the product/profile layer for which sensor profile is plugged into
//! each port. Under the `iolink-native` feature, stack-backed coverage lives in
//! the native bridge and multi-chip tests, where real `iolinki-master` ports are
//! paired with real reentrant `iolinki` device contexts.

#[derive(Debug, Clone)]
enum PortModel {
    Empty,
    Proximity { present: bool },
    Pressure { bar: f32 },
    Distance { mm: u16 },
}

#[derive(Debug)]
pub struct IolinkStation {
    ports: Vec<PortModel>,
}

impl IolinkStation {
    pub fn new_4port() -> Self {
        Self {
            ports: vec![PortModel::Empty; 4],
        }
    }

    pub fn connect_proximity(&mut self, port: usize, present: bool) {
        self.ports[port - 1] = PortModel::Proximity { present };
    }

    pub fn connect_pressure(&mut self, port: usize, bar: f32) {
        self.ports[port - 1] = PortModel::Pressure { bar };
    }

    pub fn connect_distance(&mut self, port: usize, mm: u16) {
        self.ports[port - 1] = PortModel::Distance { mm };
    }

    pub fn port_profiles(&self) -> Vec<String> {
        self.ports
            .iter()
            .map(|p| match p {
                PortModel::Empty => "empty".to_string(),
                PortModel::Proximity { present } => {
                    format!("proximity:{}", if *present { "present" } else { "clear" })
                }
                PortModel::Pressure { bar } => format!("pressure:{bar:.2}bar"),
                PortModel::Distance { mm } => format!("distance:{mm}mm"),
            })
            .collect()
    }
}
