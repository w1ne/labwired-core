// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Raw TCP/UDP socket and TLS-socket AT commands: `+QIOPEN`, `+QISTATE`,
//! `+QISEND`, `+QIRD`, `+QICLOSE`, `+QIDNSCFG`, `+QIDNSGIP`, `+QSSLCFG`,
//! `+QSSLSTATE`, `+QSSLOPEN`, `+QSSLSEND`, `+QSSLRECV`, `+QSSLCLOSE`.

use super::super::*;
use super::AtForm;

impl QuectelBg770a {
    pub(super) fn at_qiopen(&mut self, form: &AtForm<'_>, line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit(
                    "\r\n+QIOPEN: (1-5),(0-11),\"TCP/UDP/TCP LISTENER/UDP SERVICE\",\
                     \"<IP_address>/<domain_name>\",<remote_port>,<local_port>,(0-2)\r\n",
                );
                self.ok();
            }
            AtForm::Write(_) => {
                // QIOPEN write — open a raw TCP/UDP socket. Sync OK, then async URC
                // `+QIOPEN: <connectID>,<err>` (err=0 success, non-zero failure).
                // HAZARD: original check used the case-sensitive `line` prefix.
                if let Some(arg) = line.strip_prefix("AT+QIOPEN=") {
                    let parts: Vec<&str> = arg.split(',').collect();
                    // Min args: contextID, connectID, service_type, host, port.
                    if parts.len() < 5 {
                        return self.error();
                    }
                    let ctx_id = parts[0].trim().parse::<u8>().ok();
                    let connect_id = parts[1].trim().parse::<u8>().ok();
                    let service = parts[2].trim().trim_matches('"').to_string();
                    let host = parts[3].trim().trim_matches('"').to_string();
                    let port = parts[4].trim().parse::<u16>().ok();
                    return match (ctx_id, connect_id, port) {
                        (Some(1..=5), Some(cid @ 0..=11), Some(port)) => {
                            self.current_delay_us = DELAY_QIOPEN_US;
                            self.ok();
                            let result: i16 = if self.qiact_cid1 == 1 {
                                self.sockets[cid as usize] = Socket {
                                    state: SocketState::Open,
                                    service_type: service,
                                    remote_host: host,
                                    remote_port: port,
                                    rx_buffer: Vec::new(),
                                };
                                0
                            } else {
                                // 565 (Quectel: "Failed to activate a PDP context") —
                                // standard mapping for no-context attach attempts.
                                565
                            };
                            let urc = format!("\r\n+QIOPEN: {},{}\r\n", cid, result);
                            self.deferred_urcs
                                .push((URC_DELAY_QIOPEN_US, urc.into_bytes()));
                        }
                        _ => self.error(),
                    };
                }
                self.error();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qistate(&mut self, form: &AtForm<'_>, _line: &str, upper: &str) {
        match form {
            AtForm::Test => {
                // Test form: real HW returns bare OK, no payload.
                self.ok();
            }
            AtForm::Read => {
                // QISTATE — list active sockets or query one. Format:
                //   +QISTATE: <conn>,<service>,<host>,<rport>,<lport>,<state>,<ctxid>,
                //             <sring>,<access_mode>
                // Real HW returns bare OK when no sockets are open.
                let open: Vec<usize> = (0..self.sockets.len())
                    .filter(|&i| self.sockets[i].state == SocketState::Open)
                    .collect();
                for cid in open {
                    let s = &self.sockets[cid];
                    self.emit(&format!(
                        "\r\n+QISTATE: {},\"{}\",\"{}\",{},0,2,1,0,0,\"usbmodem\"\r\n",
                        cid, s.service_type, s.remote_host, s.remote_port
                    ));
                }
                self.ok();
            }
            AtForm::Write(_) => {
                if let Some(arg) = upper.strip_prefix("AT+QISTATE=") {
                    // `1,<connectID>` queries by connect_id (the only form we model).
                    let mut parts = arg.split(',');
                    let kind = parts.next().and_then(|s| s.trim().parse::<u8>().ok());
                    let cid = parts.next().and_then(|s| s.trim().parse::<u8>().ok());
                    return match (kind, cid) {
                        (Some(1), Some(c @ 0..=11)) => {
                            let s = &self.sockets[c as usize];
                            if s.state == SocketState::Open {
                                let line = format!(
                                    "\r\n+QISTATE: {},\"{}\",\"{}\",{},0,2,1,0,0,\"usbmodem\"\r\n",
                                    c, s.service_type, s.remote_host, s.remote_port
                                );
                                self.emit(&line);
                            }
                            self.ok()
                        }
                        _ => self.error(),
                    };
                }
                self.error();
            }
            AtForm::Exec => self.error(),
        }
    }

    pub(super) fn at_qisend(&mut self, form: &AtForm<'_>, line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QISEND: (0-11),(0-1460)\r\n");
                self.ok();
            }
            AtForm::Write(_) => {
                // QISEND write — enter prompt mode like QMTPUB. Two forms:
                //   AT+QISEND=<cid>          → variable length, terminated by Ctrl-Z
                //   AT+QISEND=<cid>,<len>    → fixed length, modem returns SEND OK
                //                               after exactly <len> bytes
                // HAZARD: original check used the case-sensitive `line` prefix.
                if let Some(arg) = line.strip_prefix("AT+QISEND=") {
                    let mut parts = arg.split(',');
                    let cid = parts.next().and_then(|s| s.trim().parse::<u8>().ok());
                    let len = parts.next().and_then(|s| s.trim().parse::<usize>().ok());
                    return match cid {
                        Some(c @ 0..=11) if self.sockets[c as usize].state == SocketState::Open => {
                            self.emit("\r\n> ");
                            self.awaiting_qisend_payload = Some((c, len.unwrap_or(0)));
                        }
                        _ => self.error(),
                    };
                }
                self.error();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qird(&mut self, form: &AtForm<'_>, _line: &str, upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QIRD: (0-11),(0-1500)\r\n");
                self.ok();
            }
            AtForm::Write(_) => {
                // QIRD write — read buffered data. Format:
                //   +QIRD: <read_actual_length>\r\n<data>\r\n\r\nOK\r\n
                // If no data is available, returns `+QIRD: 0` + OK.
                if let Some(arg) = upper.strip_prefix("AT+QIRD=") {
                    let mut parts = arg.split(',');
                    let cid = parts.next().and_then(|s| s.trim().parse::<u8>().ok());
                    let max_len = parts
                        .next()
                        .and_then(|s| s.trim().parse::<usize>().ok())
                        .unwrap_or(1500);
                    return match cid {
                        Some(c @ 0..=11) => {
                            let buf = &mut self.sockets[c as usize].rx_buffer;
                            let take = buf.len().min(max_len);
                            let drained: Vec<u8> = buf.drain(..take).collect();
                            let mut payload =
                                format!("\r\n+QIRD: {}\r\n", drained.len()).into_bytes();
                            payload.extend_from_slice(&drained);
                            if !drained.is_empty() {
                                payload.extend_from_slice(b"\r\n");
                            }
                            self.respond_buf.extend(payload);
                            self.ok()
                        }
                        _ => self.error(),
                    };
                }
                self.error();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qiclose(&mut self, form: &AtForm<'_>, _line: &str, upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QICLOSE: (0-11),(0-65535)\r\n");
                self.ok();
            }
            AtForm::Write(_) => {
                // QICLOSE write — close a socket.
                if let Some(arg) = upper.strip_prefix("AT+QICLOSE=") {
                    let cid = arg
                        .split(',')
                        .next()
                        .and_then(|s| s.trim().parse::<u8>().ok());
                    return match cid {
                        Some(c @ 0..=11) => {
                            self.sockets[c as usize] = Socket::default();
                            self.current_delay_us = DELAY_QICLOSE_US;
                            self.ok()
                        }
                        _ => self.error(),
                    };
                }
                self.error();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qidnscfg(&mut self, form: &AtForm<'_>, _line: &str, upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QIDNSCFG: (1-5),<pridnsaddr>,<secdnsaddr>\r\n");
                self.ok();
            }
            AtForm::Write(_) => {
                // QIDNSCFG read: errors when no context — matches real HW.
                if let Some(arg) = upper.strip_prefix("AT+QIDNSCFG=") {
                    return match arg.trim().parse::<u8>() {
                        Ok(_) if self.qiact_cid1 == 0 => self.error(),
                        Ok(1..=5) => {
                            self.emit("\r\n+QIDNSCFG: 1,\"8.8.8.8\",\"8.8.4.4\"\r\n");
                            self.ok()
                        }
                        _ => self.error(),
                    };
                }
                self.error();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qidnsgip(&mut self, form: &AtForm<'_>, line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QIDNSGIP: (1-5),<hostname>\r\n");
                self.ok();
            }
            AtForm::Write(_) => {
                // QIDNSGIP write — async DNS lookup. Sync OK, then `+QIURC: "dnsgip",
                // <err>,<count>,<ip>` URC. We always succeed with a synthetic IP.
                // HAZARD: original check used the case-sensitive `line` prefix.
                if let Some(arg) = line.strip_prefix("AT+QIDNSGIP=") {
                    let mut parts = arg.split(',');
                    let ctx_id = parts.next().and_then(|s| s.trim().parse::<u8>().ok());
                    let host = parts.next().map(|s| s.trim().trim_matches('"').to_string());
                    return match (ctx_id, host) {
                        (Some(1..=5), Some(_)) => {
                            self.ok();
                            let urc = b"\r\n+QIURC: \"dnsgip\",0,1\r\n+QIURC: \"dnsgip\",\"93.184.216.34\"\r\n"
                                .to_vec();
                            self.deferred_urcs.push((URC_DELAY_QIDNS_US, urc));
                        }
                        _ => self.error(),
                    };
                }
                self.error();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qsslcfg(&mut self, form: &AtForm<'_>, line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit(
                    "\r\n+QSSLCFG: \"sslversion\",(0-5),(0-4)\r\n\
                     +QSSLCFG: \"ciphersuite\",(0-5),(0X0035,0X002F,0X0004,0X0005,\
0X000A,0X003D,0XC002,0XC003,0XC004,0XC005,0XC007,0XC008,0XC009,0XC00A,0XC011,\
0XC012,0XC013,0XC014,0XC00C,0XC00D,0XC00E,0XC00F,0XC023,0XC024,0XC025,0XC026,\
0XC027,0XC028,0XC029,0XC02A,0XC02B,0XC02F,0XC0A8,0X00AE,0XFFFF)\r\n\
                     +QSSLCFG: \"cacert\",(0-5),<cacertpath>\r\n\
                     +QSSLCFG: \"clientcert\",(0-5),<clientcertpath>\r\n\
                     +QSSLCFG: \"clientkey\",(0-5),<clientkeypath>\r\n\
                     +QSSLCFG: \"seclevel\",(0-5),(0-3)\r\n\
                     +QSSLCFG: \"session\",(0-5),(0,1)\r\n\
                     +QSSLCFG: \"sni\",(0-5),(0,1)\r\n\
                     +QSSLCFG: \"checkhost\",(0-5),(0,1)\r\n\
                     +QSSLCFG: \"ignorelocaltime\",(0-5),(0,1)\r\n\
                     +QSSLCFG: \"negotiatetime\",(0-5),(10-300)\r\n\
                     +QSSLCFG: \"renegotiation\",(0-5),(0,1)\r\n\
                     +QSSLCFG: \"dtls\",(0-5),(0-1)\r\n\
                     +QSSLCFG: \"dtlsversion\",(0-5),(0-2)\r\n",
                );
                self.ok();
            }
            AtForm::Write(_) => {
                // HAZARD: original check used the case-sensitive `line` prefix.
                if let Some(args) = line.strip_prefix("AT+QSSLCFG=") {
                    if let Some((key, rest)) = parse_quoted_subkey(args) {
                        let key_lower = key.to_ascii_lowercase();
                        let ctxid = rest.first().and_then(|s| s.parse::<u8>().ok());
                        let value = rest.get(1);
                        return match (ctxid, value) {
                            (Some(c), None) if c <= 5 => {
                                // Read form. Real HW returns the current value for any
                                // sub-key; we only persist `seclevel`, so others get a
                                // captured-default placeholder.
                                if key_lower == "seclevel" {
                                    self.emit(&format!(
                                        "\r\n+QSSLCFG: \"seclevel\",{},{}\r\n",
                                        c, self.ssl_seclevel[c as usize]
                                    ));
                                }
                                self.ok()
                            }
                            (Some(c), Some(v)) if c <= 5 => {
                                if key_lower == "seclevel" {
                                    if let Ok(n) = v.parse::<u8>() {
                                        if n <= 3 {
                                            self.ssl_seclevel[c as usize] = n;
                                        }
                                    }
                                }
                                self.ok()
                            }
                            _ => self.error(),
                        };
                    }
                    return self.error();
                }
                self.error();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qsslstate(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Read | AtForm::Test => self.ok(),
            _ => self.error(),
        }
    }

    pub(super) fn at_qsslopen(&mut self, form: &AtForm<'_>, line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QSSLOPEN: (1-5),(0-5),(0-11),<serveraddr>,<server_port>,(0-2)\r\n");
                self.ok();
            }
            AtForm::Write(_) => {
                // Write forms mirror QIOPEN / QISEND / QIRD / QICLOSE — same socket
                // table is reused so a SSL-opened connectID shows up in QISTATE too.
                // HAZARD: original check used the case-sensitive `line` prefix.
                if let Some(arg) = line.strip_prefix("AT+QSSLOPEN=") {
                    let parts: Vec<&str> = arg.split(',').collect();
                    if parts.len() < 5 {
                        return self.error();
                    }
                    let ctx_id = parts[0].trim().parse::<u8>().ok();
                    let _ssl_ctx = parts[1].trim().parse::<u8>().ok();
                    let connect_id = parts[2].trim().parse::<u8>().ok();
                    let host = parts[3].trim().trim_matches('"').to_string();
                    let port = parts[4].trim().parse::<u16>().ok();
                    return match (ctx_id, connect_id, port) {
                        (Some(1..=5), Some(cid @ 0..=11), Some(port)) => {
                            self.current_delay_us = DELAY_QIOPEN_US;
                            self.ok();
                            let result: i16 = if self.qiact_cid1 == 1 {
                                self.sockets[cid as usize] = Socket {
                                    state: SocketState::Open,
                                    service_type: "SSL".to_string(),
                                    remote_host: host,
                                    remote_port: port,
                                    rx_buffer: Vec::new(),
                                };
                                0
                            } else {
                                565
                            };
                            let urc = format!("\r\n+QSSLOPEN: {},{}\r\n", cid, result);
                            self.deferred_urcs
                                .push((URC_DELAY_QIOPEN_US, urc.into_bytes()));
                        }
                        _ => self.error(),
                    };
                }
                self.error();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qsslsend(&mut self, form: &AtForm<'_>, line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QSSLSEND: (0-11)[,(1-1460)]\r\n");
                self.ok();
            }
            AtForm::Write(_) => {
                // HAZARD: original check used the case-sensitive `line` prefix.
                if let Some(arg) = line.strip_prefix("AT+QSSLSEND=") {
                    let mut parts = arg.split(',');
                    let cid = parts.next().and_then(|s| s.trim().parse::<u8>().ok());
                    let len = parts.next().and_then(|s| s.trim().parse::<usize>().ok());
                    return match cid {
                        Some(c @ 0..=11) if self.sockets[c as usize].state == SocketState::Open => {
                            self.emit("\r\n> ");
                            self.awaiting_qisend_payload = Some((c, len.unwrap_or(0)));
                        }
                        _ => self.error(),
                    };
                }
                self.error();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qsslrecv(&mut self, form: &AtForm<'_>, _line: &str, upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QSSLRECV: (0-11),(1-1500)\r\n");
                self.ok();
            }
            AtForm::Write(_) => {
                if let Some(arg) = upper.strip_prefix("AT+QSSLRECV=") {
                    let mut parts = arg.split(',');
                    let cid = parts.next().and_then(|s| s.trim().parse::<u8>().ok());
                    let max_len = parts
                        .next()
                        .and_then(|s| s.trim().parse::<usize>().ok())
                        .unwrap_or(1500);
                    return match cid {
                        Some(c @ 0..=11) => {
                            let buf = &mut self.sockets[c as usize].rx_buffer;
                            let take = buf.len().min(max_len);
                            let drained: Vec<u8> = buf.drain(..take).collect();
                            let mut payload =
                                format!("\r\n+QSSLRECV: {}\r\n", drained.len()).into_bytes();
                            payload.extend_from_slice(&drained);
                            if !drained.is_empty() {
                                payload.extend_from_slice(b"\r\n");
                            }
                            self.respond_buf.extend(payload);
                            self.ok()
                        }
                        _ => self.error(),
                    };
                }
                self.error();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qsslclose(&mut self, form: &AtForm<'_>, _line: &str, upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QSSLCLOSE: (0-11)\r\n");
                self.ok();
            }
            AtForm::Write(_) => {
                if let Some(arg) = upper.strip_prefix("AT+QSSLCLOSE=") {
                    let cid = arg
                        .split(',')
                        .next()
                        .and_then(|s| s.trim().parse::<u8>().ok());
                    return match cid {
                        Some(c @ 0..=11) => {
                            self.sockets[c as usize] = Socket::default();
                            self.current_delay_us = DELAY_QICLOSE_US;
                            self.ok()
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
