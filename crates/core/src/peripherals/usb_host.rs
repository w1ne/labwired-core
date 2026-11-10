// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The scripted USB host shared by the device-controller models
//! ([`crate::peripherals::imxrt::usb`], [`crate::peripherals::wch_usbfs`]).
//!
//! A model owns the wire protocol of its controller; this module owns what
//! the host asks for and what it learned: the control requests ([`Setup`]),
//! the script steps ([`HostStep`]), the enumeration an OS performs
//! ([`enumeration_script`] plus the requests [`after_control`] derives from
//! the answers), and the [`UsbHostLog`] a test reads.

use std::collections::VecDeque;

/// One standard/class control request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setup {
    pub bm_request_type: u8,
    pub b_request: u8,
    pub w_value: u16,
    pub w_index: u16,
    pub w_length: u16,
    /// Host -> device data stage (OUT); empty for IN or no-data requests.
    pub data: Vec<u8>,
}

impl Setup {
    pub fn get_descriptor(kind: u8, index: u8, lang: u16, len: u16) -> Self {
        Self {
            bm_request_type: 0x80,
            b_request: 6,
            w_value: ((kind as u16) << 8) | index as u16,
            w_index: lang,
            w_length: len,
            data: Vec::new(),
        }
    }
    /// The 8-byte SETUP packet.
    pub fn bytes(&self) -> [u8; 8] {
        let v = self.w_value.to_le_bytes();
        let i = self.w_index.to_le_bytes();
        let l = self.w_length.to_le_bytes();
        [
            self.bm_request_type,
            self.b_request,
            v[0],
            v[1],
            i[0],
            i[1],
            l[0],
            l[1],
        ]
    }
    /// Device-to-host data stage.
    pub fn dir_in(&self) -> bool {
        self.bm_request_type & 0x80 != 0
    }
}

/// A step of the host script.
#[derive(Debug, Clone)]
pub enum HostStep {
    Control(Setup),
    /// Interrupt/bulk OUT data to endpoint `ep` (1..7).
    Out {
        ep: u8,
        data: Vec<u8>,
    },
    /// Read up to `max` bytes from IN endpoint `ep` (one transfer).
    In {
        ep: u8,
        max: usize,
    },
    /// Leave the bus idle for `us` microseconds before the next step.
    Wait {
        us: u64,
    },
}

/// What the host observed.
#[derive(Debug, Default, Clone)]
pub struct UsbHostLog {
    /// Bus resets the host issued.
    pub resets: u32,
    /// (setup, data returned / sent, completed)
    pub control: Vec<(Setup, Vec<u8>, bool)>,
    /// (endpoint, data) of completed IN transfers on non-control endpoints.
    pub ins: Vec<(u8, Vec<u8>)>,
    /// (endpoint, bytes accepted) of completed OUT transfers.
    pub outs: Vec<(u8, usize)>,
    /// Address assigned with SET_ADDRESS.
    pub address: Option<u8>,
    pub device_descriptor: Vec<u8>,
    pub config_descriptor: Vec<u8>,
    /// String descriptors decoded to text, by index.
    pub strings: Vec<(u8, String)>,
    /// Human-readable progress / failure notes.
    pub notes: Vec<String>,
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

impl UsbHostLog {
    /// The log as text lines, for the `host` peripheral log. Facts first
    /// (device IDs, address, interfaces, strings), then every transfer in
    /// order, then the notes:
    ///
    /// ```text
    /// device 1fc9:0135 class 00/00/00 usb 0200 ep0 64
    /// address 5
    /// interface 0 alt 0 class 03/00/00 endpoints 2
    /// endpoint 0x81 attr 0x03 max 64
    /// string 2 "Product"
    /// control 80 06 0100 0000 0012 ok 12 01 00 02 ...
    /// in 1 01 02
    /// out 2 64
    /// note ...
    /// ```
    pub fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if self.resets > 0 {
            lines.push(format!("resets {}", self.resets));
        }
        let dd = &self.device_descriptor;
        if dd.len() >= 18 {
            lines.push(format!(
                "device {:04x}:{:04x} class {:02x}/{:02x}/{:02x} usb {:04x} ep0 {}",
                u16::from_le_bytes([dd[8], dd[9]]),
                u16::from_le_bytes([dd[10], dd[11]]),
                dd[4],
                dd[5],
                dd[6],
                u16::from_le_bytes([dd[2], dd[3]]),
                dd[7]
            ));
        }
        if let Some(address) = self.address {
            lines.push(format!("address {address}"));
        }
        let cfg = &self.config_descriptor;
        let mut i = 0;
        while i + 1 < cfg.len() && cfg[i] >= 2 {
            let d = &cfg[i..(i + cfg[i] as usize).min(cfg.len())];
            match d[1] {
                4 if d.len() >= 9 => lines.push(format!(
                    "interface {} alt {} class {:02x}/{:02x}/{:02x} endpoints {}",
                    d[2], d[3], d[5], d[6], d[7], d[4]
                )),
                5 if d.len() >= 7 => lines.push(format!(
                    "endpoint {:#04x} attr {:#04x} max {}",
                    d[2],
                    d[3],
                    u16::from_le_bytes([d[4], d[5]])
                )),
                _ => {}
            }
            i += cfg[i] as usize;
        }
        for (index, text) in &self.strings {
            lines.push(format!("string {index} {text:?}"));
        }
        for (setup, data, completed) in &self.control {
            let status = if *completed { "ok" } else { "incomplete" };
            let mut line = format!(
                "control {:02x} {:02x} {:04x} {:04x} {:04x} {status}",
                setup.bm_request_type,
                setup.b_request,
                setup.w_value,
                setup.w_index,
                setup.w_length
            );
            if !data.is_empty() {
                line.push(' ');
                line.push_str(&hex_bytes(data));
            }
            lines.push(line);
        }
        for (ep, data) in &self.ins {
            lines.push(format!("in {ep} {}", hex_bytes(data)));
        }
        for (ep, n) in &self.outs {
            lines.push(format!("out {ep} {n}"));
        }
        for note in &self.notes {
            lines.push(format!("note {note}"));
        }
        lines
    }
}

/// The enumeration an OS performs. Configuration-dependent requests (full
/// config length, strings, SET_CONFIGURATION) are appended by
/// [`after_control`] as answers arrive.
pub fn enumeration_script() -> VecDeque<HostStep> {
    VecDeque::from(vec![
        HostStep::Control(Setup::get_descriptor(1, 0, 0, 64)),
        HostStep::Control(Setup {
            bm_request_type: 0x00,
            b_request: 5, // SET_ADDRESS
            w_value: 5,
            w_index: 0,
            w_length: 0,
            data: Vec::new(),
        }),
        HostStep::Control(Setup::get_descriptor(1, 0, 0, 18)),
        HostStep::Control(Setup::get_descriptor(2, 0, 0, 9)),
    ])
}

/// Record a completed control transfer (`data` is what the data stage
/// carried) and queue on `script` the requests its answer implies.
pub fn after_control(
    log: &mut UsbHostLog,
    script: &mut VecDeque<HostStep>,
    setup: Setup,
    data: Vec<u8>,
) {
    if setup.b_request == 5 && setup.bm_request_type == 0 {
        log.address = Some(setup.w_value as u8);
    }
    if setup.b_request == 6 && setup.bm_request_type == 0x80 {
        let kind = (setup.w_value >> 8) as u8;
        let index = setup.w_value as u8;
        match kind {
            1 => {
                log.device_descriptor = data.clone();
                if setup.w_length == 18 && data.len() >= 18 {
                    // Strings the device announced, then nothing else yet.
                    let mut strings: Vec<u8> = [data[14], data[15], data[16]]
                        .into_iter()
                        .filter(|&i| i != 0)
                        .collect();
                    strings.dedup();
                    if !strings.is_empty() {
                        script.push_back(HostStep::Control(Setup::get_descriptor(3, 0, 0, 255)));
                        for i in strings {
                            script.push_back(HostStep::Control(Setup::get_descriptor(
                                3, i, 0x0409, 255,
                            )));
                        }
                    }
                }
            }
            2 => {
                if setup.w_length == 9 && data.len() >= 4 {
                    let total = u16::from_le_bytes([data[2], data[3]]);
                    script.push_front(HostStep::Control(Setup::get_descriptor(2, 0, 0, total)));
                } else {
                    log.config_descriptor = data.clone();
                    let cfg = data.get(5).copied().unwrap_or(1);
                    script.push_back(HostStep::Control(Setup {
                        bm_request_type: 0,
                        b_request: 9, // SET_CONFIGURATION
                        w_value: cfg as u16,
                        w_index: 0,
                        w_length: 0,
                        data: Vec::new(),
                    }));
                }
            }
            3 if index != 0 && data.len() >= 2 => {
                let units: Vec<u16> = data[2..]
                    .chunks(2)
                    .filter(|c| c.len() == 2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect();
                log.strings.push((index, String::from_utf16_lossy(&units)));
            }
            _ => {}
        }
    }
    log.control.push((setup, data, true));
}

fn parse_hex_bytes(text: &str) -> Result<Vec<u8>, String> {
    let digits: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if digits.len() % 2 != 0 {
        return Err(format!("odd number of hex digits in {text:?}"));
    }
    (0..digits.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&digits[i..i + 2], 16).map_err(|e| format!("{text:?}: {e}")))
        .collect()
}

/// Host script steps from a chip descriptor's `host_steps:` list, run after
/// enumeration. Each entry is one of
///
/// ```yaml
/// - control: "21 0a 0000 0001 0000"  # bmRequestType bRequest wValue wIndex wLength
///   data: "aa 01 02"                  # OUT data stage (optional, hex)
/// - in: 1                             # one IN transfer from endpoint 1
///   max: 8                            # wMaxPacketSize asked for
/// - wait_us: 1000                     # bus idle
/// ```
///
/// The `control` field is the same five fields, in the same order, as a
/// `control` line of [`UsbHostLog::lines`].
pub fn parse_host_steps(value: &serde_yaml::Value) -> Result<Vec<HostStep>, String> {
    let list = value
        .as_sequence()
        .ok_or_else(|| "host_steps must be a list".to_string())?;
    list.iter()
        .map(|entry| {
            if let Some(text) = entry.get("control").and_then(|v| v.as_str()) {
                let fields: Vec<&str> = text.split_whitespace().collect();
                let num = |i: usize| -> Result<u16, String> {
                    let f = fields
                        .get(i)
                        .ok_or_else(|| format!("control {text:?}: 5 fields"))?;
                    u16::from_str_radix(f, 16).map_err(|e| format!("control {text:?}: {e}"))
                };
                if fields.len() != 5 {
                    return Err(format!("control {text:?}: expected 5 hex fields"));
                }
                let data = match entry.get("data").and_then(|v| v.as_str()) {
                    Some(hex) => parse_hex_bytes(hex)?,
                    None => Vec::new(),
                };
                let setup = Setup {
                    bm_request_type: num(0)? as u8,
                    b_request: num(1)? as u8,
                    w_value: num(2)?,
                    w_index: num(3)?,
                    w_length: num(4)?,
                    data,
                };
                if !setup.dir_in() && setup.data.len() != setup.w_length as usize {
                    return Err(format!(
                        "control {text:?}: OUT data is {} bytes, wLength {}",
                        setup.data.len(),
                        setup.w_length
                    ));
                }
                Ok(HostStep::Control(setup))
            } else if let Some(ep) = entry.get("in").and_then(|v| v.as_u64()) {
                let max = entry.get("max").and_then(|v| v.as_u64()).unwrap_or(64) as usize;
                Ok(HostStep::In { ep: ep as u8, max })
            } else if let Some(us) = entry.get("wait_us").and_then(|v| v.as_u64()) {
                Ok(HostStep::Wait { us })
            } else {
                Err(format!("unknown host step {entry:?}"))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_steps_parse_control_in_and_wait() {
        let yaml: serde_yaml::Value = serde_yaml::from_str(
            r#"
- control: "21 09 03aa 0000 0003"
  data: "aa 01 02"
- control: "a1 01 03aa 0000 00ff"
- in: 2
  max: 8
- wait_us: 500
"#,
        )
        .unwrap();
        let steps = parse_host_steps(&yaml).unwrap();
        assert_eq!(steps.len(), 4);
        match &steps[0] {
            HostStep::Control(s) => {
                assert_eq!(s.bytes(), [0x21, 0x09, 0xaa, 0x03, 0, 0, 3, 0]);
                assert_eq!(s.data, [0xaa, 1, 2]);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(&steps[1], HostStep::Control(s) if s.dir_in() && s.w_length == 255));
        assert!(matches!(steps[2], HostStep::In { ep: 2, max: 8 }));
        assert!(matches!(steps[3], HostStep::Wait { us: 500 }));
    }

    #[test]
    fn host_steps_reject_short_out_data() {
        let yaml: serde_yaml::Value =
            serde_yaml::from_str(r#"[{ control: "21 09 03aa 0000 0004", data: "aa" }]"#).unwrap();
        assert!(parse_host_steps(&yaml).unwrap_err().contains("wLength 4"));
    }
}
