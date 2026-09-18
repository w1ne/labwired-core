// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Quectel MQTT AT commands: `+QMTCFG`, `+QMTOPEN`, `+QMTCONN`, `+QMTPUB`,
//! `+QMTSUB`, `+QMTDISC`, `+QMTCLOSE`.
//!
//! The MQTT engine is async: write commands return OK immediately and the
//! operation outcome comes later as a `+QMTOPEN/CONN/PUB/...` URC.
//! Status codes are from the Quectel MQTT Application Note:
//!   +QMTOPEN: <id>,<r>    0 = success, 3 = PDP activation failed
//!   +QMTCONN: <id>,<r>,<rc>  r=0 success, rc=0 accepted
//!   +QMTPUB:  <id>,<msgid>,<r>  r=0 success
//!   +QMTDISC: <id>,<r>    0 = success
//!   +QMTCLOSE: <id>,<r>   0 = success

use super::super::*;
use super::AtForm;

impl QuectelBg770a {
    pub(super) fn at_qmtcfg(&mut self, form: &AtForm<'_>, line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit(
                    "\r\n+QMTCFG: \"version\",(0-5),(3,4)\r\n\
                     +QMTCFG: \"pdpcid\",(0-5),(1-5)\r\n\
                     +QMTCFG: \"ssl\",(0-5),(0,1),(0-5)\r\n\
                     +QMTCFG: \"keepalive\",(0-5),(0-3600)\r\n\
                     +QMTCFG: \"session\",(0-5),(0,1)\r\n\
                     +QMTCFG: \"timeout\",(0-5),(1-60),(0-10),(0,1)\r\n\
                     +QMTCFG: \"will\",(0-5),(0,1),(0-2),(0,1),<will_topic>,<will_message>\r\n\
                     +QMTCFG: \"recv/mode\",(0-5),(0,1),(0,1)\r\n\
                     +QMTCFG: \"aliauth\",(0-5),<product_key>,<device_name>,<device_secret>\r\n",
                );
                self.ok();
            }
            AtForm::Write(_) => {
                // QMTCFG="ssl",<client>[,<enable>,<ssl_ctxid>] — read/write the SSL
                // toggle for an MQTT client. Read with one arg returns:
                //   `+QMTCFG: "ssl",0`        when SSL is disabled
                //   `+QMTCFG: "ssl",1,<ctx>`  when SSL is enabled
                // Write with three args sets enable+ctxid. Captured from real HW.
                // HAZARD: original check used the case-sensitive `line` prefix.
                if let Some(args) = line.strip_prefix("AT+QMTCFG=") {
                    if let Some((key, rest)) = parse_quoted_subkey(args) {
                        let key_lower = key.to_ascii_lowercase();
                        let nums: Vec<u8> =
                            rest.iter().filter_map(|s| s.parse::<u8>().ok()).collect();
                        if key_lower == "ssl" {
                            return match nums.len() {
                                1 => {
                                    let id = nums[0] as usize;
                                    if id > 5 {
                                        return self.error();
                                    }
                                    if self.mqtt[id].ssl_enabled {
                                        self.emit(&format!(
                                            "\r\n+QMTCFG: \"ssl\",1,{}\r\n",
                                            self.mqtt[id].ssl_ctxid
                                        ));
                                    } else {
                                        self.emit("\r\n+QMTCFG: \"ssl\",0\r\n");
                                    }
                                    self.ok()
                                }
                                3 => {
                                    let id = nums[0] as usize;
                                    if id > 5 || nums[1] > 1 || nums[2] > 5 {
                                        return self.error();
                                    }
                                    self.mqtt[id].ssl_enabled = nums[1] == 1;
                                    self.mqtt[id].ssl_ctxid = nums[2];
                                    self.ok()
                                }
                                _ => self.error(),
                            };
                        }
                        // Any other QMTCFG sub-key: accept the write as a no-op so
                        // firmware boot scripts proceed. Real-HW validates the args,
                        // but for happy-path simulation a permissive OK is enough.
                        return self.ok();
                    }
                    return self.ok();
                }
                self.error();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qmtopen(&mut self, form: &AtForm<'_>, line: &str, upper: &str) {
        match form {
            // The Quectel MQTT/socket test forms emit a SINGLE \r\n between the
            // last `+...` payload line and the final `OK` — not the standard two.
            // We mirror this quirk exactly.
            AtForm::Test => {
                self.emit("\r\n+QMTOPEN: (0-5),<host_name>,(0-65535)\r\n");
                self.ok_compact();
            }
            AtForm::Read => {
                // No clients open → bare OK (no payload).
                let open: Vec<usize> = (0..self.mqtt.len())
                    .filter(|&i| self.mqtt[i].state != MqttState::Closed)
                    .collect();
                for id in open {
                    self.emit(&format!("\r\n+QMTOPEN: {},\"broker\",1883\r\n", id));
                }
                self.ok();
            }
            AtForm::Write(_) => {
                // QMTOPEN write — open MQTT network. Returns OK immediately, then
                // a `+QMTOPEN: <id>,<result>` URC. With no active PDP the result is 3
                // (PDP failed), matching real HW.
                // Form: AT+QMTOPEN=<id>,"host",port  (use original `line` for host quotes)
                if upper.starts_with("AT+QMTOPEN=") {
                    let arg = line
                        .strip_prefix("AT+QMTOPEN=")
                        .or_else(|| {
                            // line may be mixed-case; fall back to upper without host fidelity
                            upper.strip_prefix("AT+QMTOPEN=")
                        })
                        .unwrap_or("");
                    let mut parts = arg.splitn(3, ',');
                    let client_id = parts.next().and_then(|s| s.trim().parse::<u8>().ok());
                    let host_raw = parts.next().unwrap_or("").trim();
                    let host = host_raw.trim_matches('"');
                    let port = parts
                        .next()
                        .and_then(|s| s.trim().parse::<u16>().ok())
                        .unwrap_or(1883);
                    return match client_id {
                        Some(id @ 0..=5) => {
                            self.current_delay_us = DELAY_QMTOPEN_US;
                            self.ok();
                            // Result codes (Quectel-shaped): 0 ok, 1 open fail, 3 PDP fail.
                            // RF path-loss gates open: no service (CSQ 99) cannot open MQTT.
                            let result: i8 = if self.qiact_cid1 != 1 {
                                3
                            } else if !self.rf_link_ok() {
                                1
                            } else {
                                0
                            };
                            if result == 0 {
                                self.mqtt[id as usize].state = MqttState::Initialized;
                                self.mqtt[id as usize].broker_host = host.to_string();
                                self.mqtt[id as usize].broker_port = port;
                                self.mqtt_net.open(&self.mqtt_endpoint_id(), id, host, port);
                            }
                            let urc = format!("\r\n+QMTOPEN: {},{}\r\n", id, result);
                            self.deferred_urcs
                                .push((URC_DELAY_QMTOPEN_US, urc.into_bytes()));
                        }
                        _ => self.error(),
                    };
                }
                self.error();
            }
            AtForm::Exec => self.error(),
        }
    }

    pub(super) fn at_qmtconn(&mut self, form: &AtForm<'_>, _line: &str, upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QMTCONN: (0-5),<clientID>,<username>,<password>\r\n");
                self.ok_compact();
            }
            AtForm::Read => {
                let connected: Vec<usize> = (0..self.mqtt.len())
                    .filter(|&i| self.mqtt[i].state == MqttState::Connected)
                    .collect();
                for id in connected {
                    self.emit(&format!("\r\n+QMTCONN: {},3\r\n", id));
                }
                self.ok();
            }
            AtForm::Write(_) => {
                if let Some(arg) = upper.strip_prefix("AT+QMTCONN=") {
                    let client_id = arg
                        .split(',')
                        .next()
                        .and_then(|s| s.trim().parse::<u8>().ok());
                    return match client_id {
                        Some(id @ 0..=5) => {
                            if self.mqtt[id as usize].state != MqttState::Initialized {
                                return self.error();
                            }
                            self.current_delay_us = DELAY_QMTCONN_US;
                            self.ok();
                            // +QMTCONN: <id>,<result>,<ret_code> — result 0 = accepted.
                            let (result, ret) = if self.rf_link_ok() {
                                self.mqtt[id as usize].state = MqttState::Connected;
                                self.mqtt_net.connect(&self.mqtt_endpoint_id(), id);
                                (0, 0)
                            } else {
                                // Stay Initialized; broker connect refused without RF.
                                (1, 1)
                            };
                            let urc = format!("\r\n+QMTCONN: {},{},{}\r\n", id, result, ret);
                            self.deferred_urcs
                                .push((URC_DELAY_QMTCONN_US, urc.into_bytes()));
                        }
                        _ => self.error(),
                    };
                }
                self.error();
            }
            AtForm::Exec => self.error(),
        }
    }

    pub(super) fn at_qmtpub(&mut self, form: &AtForm<'_>, line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QMTPUB: (0-5),(0-65535),(0-2),(0,1),<topic>,(1-4096)\r\n");
                self.ok_compact();
            }
            AtForm::Write(_) => {
                // QMTPUB write — publish. Real HW: emits `> ` prompt, firmware sends
                // payload + 0x1A, modem emits OK + async `+QMTPUB: <id>,<msgid>,<r>`.
                // Form: AT+QMTPUB=<id>,<msgId>,<qos>,<retain>,"topic"[,len]
                // HAZARD: original check used the case-sensitive `line` prefix.
                if let Some(arg) = line.strip_prefix("AT+QMTPUB=") {
                    // Parse with awareness of quoted topic.
                    let mut client_id: Option<u8> = None;
                    let mut msg_id: Option<u16> = None;
                    let mut topic = String::from("topic");
                    let mut field = String::new();
                    let mut in_quote = false;
                    let mut fields: Vec<String> = Vec::new();
                    for ch in arg.chars() {
                        match ch {
                            '"' => in_quote = !in_quote,
                            ',' if !in_quote => {
                                fields.push(std::mem::take(&mut field));
                            }
                            c => field.push(c),
                        }
                    }
                    if !field.is_empty() || arg.ends_with(',') {
                        fields.push(field);
                    }
                    if !fields.is_empty() {
                        client_id = fields[0].trim().parse().ok();
                    }
                    if fields.len() > 1 {
                        msg_id = fields[1].trim().parse().ok();
                    }
                    // fields: id, msgid, qos, retain, topic, [len]
                    if fields.len() > 4 {
                        topic = fields[4].trim().trim_matches('"').to_string();
                    }
                    return match (client_id, msg_id) {
                        (Some(id @ 0..=5), Some(mid)) => {
                            if self.mqtt[id as usize].state != MqttState::Connected {
                                return self.error();
                            }
                            self.emit("\r\n> ");
                            self.awaiting_qmtpub_payload = Some((id, mid, topic));
                            self.qmtpub_payload_buf.clear();
                        }
                        _ => self.error(),
                    };
                }
                self.error();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qmtsub(&mut self, form: &AtForm<'_>, line: &str, _upper: &str) {
        match form {
            AtForm::Write(_) => {
                // QMTSUB — subscribe. Form: AT+QMTSUB=<id>,<msgId>,"topic",qos
                // HAZARD: original check used the case-sensitive `line` prefix.
                if let Some(arg) = line.strip_prefix("AT+QMTSUB=") {
                    let mut in_quote = false;
                    let mut field = String::new();
                    let mut fields: Vec<String> = Vec::new();
                    for ch in arg.chars() {
                        match ch {
                            '"' => in_quote = !in_quote,
                            ',' if !in_quote => fields.push(std::mem::take(&mut field)),
                            c => field.push(c),
                        }
                    }
                    if !field.is_empty() {
                        fields.push(field);
                    }
                    let client_id = fields.first().and_then(|s| s.trim().parse::<u8>().ok());
                    let topic = fields
                        .get(2)
                        .map(|s| s.trim().trim_matches('"').to_string())
                        .unwrap_or_default();
                    return match client_id {
                        Some(id @ 0..=5)
                            if self.mqtt[id as usize].state == MqttState::Connected =>
                        {
                            if !topic.is_empty() {
                                self.mqtt_net
                                    .subscribe(&self.mqtt_endpoint_id(), id, &topic);
                            }
                            self.ok();
                            let urc = format!("\r\n+QMTSUB: {},1,0,0\r\n", id);
                            self.deferred_urcs
                                .push((URC_DELAY_QMTPUB_US, urc.into_bytes()));
                        }
                        _ => self.error(),
                    };
                }
                self.error();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qmtdisc(&mut self, form: &AtForm<'_>, _line: &str, upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QMTDISC: (0-5)\r\n");
                self.ok_compact();
            }
            AtForm::Write(_) => {
                // QMTDISC — graceful disconnect.
                if let Some(arg) = upper.strip_prefix("AT+QMTDISC=") {
                    let client_id = arg.trim().parse::<u8>().ok();
                    return match client_id {
                        Some(id @ 0..=5)
                            if self.mqtt[id as usize].state == MqttState::Connected =>
                        {
                            self.ok();
                            self.mqtt[id as usize].state = MqttState::Initialized;
                            self.mqtt_net.disconnect(&self.mqtt_endpoint_id(), id);
                            let urc = format!("\r\n+QMTDISC: {},0\r\n", id);
                            self.deferred_urcs
                                .push((URC_DELAY_QMTDISC_US, urc.into_bytes()));
                        }
                        _ => self.error(),
                    };
                }
                self.error();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qmtclose(&mut self, form: &AtForm<'_>, _line: &str, upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QMTCLOSE: (0-5)\r\n");
                self.ok_compact();
            }
            AtForm::Write(_) => {
                // QMTCLOSE — close MQTT network connection.
                if let Some(arg) = upper.strip_prefix("AT+QMTCLOSE=") {
                    let client_id = arg.trim().parse::<u8>().ok();
                    return match client_id {
                        Some(id @ 0..=5) if self.mqtt[id as usize].state != MqttState::Closed => {
                            self.ok();
                            self.mqtt[id as usize].state = MqttState::Closed;
                            let urc = format!("\r\n+QMTCLOSE: {},0\r\n", id);
                            self.deferred_urcs
                                .push((URC_DELAY_QMTDISC_US, urc.into_bytes()));
                        }
                        _ => self.error(),
                    };
                }
                self.error();
            }
            _ => self.error(),
        }
    }
}
