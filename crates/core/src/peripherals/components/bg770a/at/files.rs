// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Quectel user-file-system AT commands: `+QFLDS`, `+QFLST`, `+QFUPL`,
//! `+QFDWL`, `+QFOPEN`, `+QFREAD`, `+QFWRITE`, `+QFCLOSE`, `+QFDEL`.

use super::super::*;
use super::AtForm;

impl QuectelBg770a {
    pub(super) fn at_qflds(&mut self, form: &AtForm<'_>, line: &str, _upper: &str) {
        match form {
            AtForm::Test => self.ok(),
            AtForm::Write(_) => {
                // QFLDS=<storage> — free/total bytes for "UFS" (User File System).
                // HAZARD: original check used the case-sensitive `line` prefix.
                if let Some(arg) = line.strip_prefix("AT+QFLDS=") {
                    let storage = arg.trim().trim_matches('"');
                    return match storage {
                        "UFS" => {
                            let used: u32 = self.filesystem.values().map(|v| v.len() as u32).sum();
                            let total: u32 = 3_776_512; // matches real-hw default capacity.
                            let free = total.saturating_sub(used);
                            self.emit(&format!("\r\n+QFLDS: {},{}\r\n", free, total));
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

    pub(super) fn at_qflst(&mut self, form: &AtForm<'_>, line: &str, _upper: &str) {
        match form {
            AtForm::Test => self.ok(),
            AtForm::Write(_) => {
                // QFLST="<pattern>" — directory listing. We support the bench's
                // default `"*"` wildcard and exact-name matches.
                // HAZARD: original check used the case-sensitive `line` prefix.
                if let Some(arg) = line.strip_prefix("AT+QFLST=") {
                    let pattern = arg.trim().trim_matches('"');
                    if pattern == "*" {
                        if self.filesystem.is_empty() {
                            self.emit("\r\n+QFLST: \"security/\",0\r\n");
                        } else {
                            let snapshot: Vec<(String, usize)> = self
                                .filesystem
                                .iter()
                                .map(|(k, v)| (k.clone(), v.len()))
                                .collect();
                            for (name, len) in snapshot {
                                self.emit(&format!("\r\n+QFLST: \"{}\",{}\r\n", name, len));
                            }
                        }
                    } else if let Some(len) = self.filesystem.get(pattern).map(|v| v.len()) {
                        self.emit(&format!("\r\n+QFLST: \"{}\",{}\r\n", pattern, len));
                    }
                    return self.ok();
                }
                self.error();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qfupl(&mut self, form: &AtForm<'_>, line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QFUPL: <filename>[,(1-<freesize>)[,(1-65535)[,(0,1)]]]\r\n");
                self.ok();
            }
            AtForm::Write(_) => {
                // QFUPL=<name>,<size>[,<timeout>[,<ack>]] — CONNECT-prompt then bytes.
                // HAZARD: original check used the case-sensitive `line` prefix.
                if let Some(arg) = line.strip_prefix("AT+QFUPL=") {
                    let parts: Vec<&str> = arg.split(',').collect();
                    if parts.len() < 2 {
                        return self.error();
                    }
                    let name = parts[0].trim().trim_matches('"').to_string();
                    let size = parts[1].trim().parse::<usize>().ok();
                    return match size {
                        Some(n) if n > 0 && n <= 1_024_000 => {
                            self.emit("\r\nCONNECT\r\n");
                            self.awaiting_qfupl = Some((name, n));
                            self.qfupl_buf.clear();
                        }
                        _ => self.error(),
                    };
                }
                self.error();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qfdwl(&mut self, form: &AtForm<'_>, line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QFDWL: <filename>\r\n");
                self.ok();
            }
            AtForm::Write(_) => {
                // QFDWL=<name> — emits CONNECT, file bytes, then `+QFDWL: <len>,<crc>`
                // and OK. We always emit a synthetic CRC of 0 for the simulated FS.
                // HAZARD: original check used the case-sensitive `line` prefix.
                if let Some(arg) = line.strip_prefix("AT+QFDWL=") {
                    let name = arg.trim().trim_matches('"').to_string();
                    return match self.filesystem.get(&name).cloned() {
                        Some(data) => {
                            let mut payload = b"\r\nCONNECT\r\n".to_vec();
                            payload.extend_from_slice(&data);
                            payload.extend_from_slice(
                                format!("\r\nOK\r\n\r\n+QFDWL: {},0\r\n", data.len()).as_bytes(),
                            );
                            self.schedule(DELAY_DEFAULT_US, payload);
                        }
                        None => self.cme_error(409), // "file does not exist" per manual.
                    };
                }
                self.error();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qfopen(&mut self, form: &AtForm<'_>, line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QFOPEN: <filename>[,(0-3)]\r\n");
                self.ok();
            }
            AtForm::Write(_) => {
                // QFOPEN=<name>,<mode> — open file handle. Returns `+QFOPEN: <handle>`.
                // HAZARD: original check used the case-sensitive `line` prefix.
                if let Some(arg) = line.strip_prefix("AT+QFOPEN=") {
                    let parts: Vec<&str> = arg.split(',').collect();
                    if parts.is_empty() {
                        return self.error();
                    }
                    let name = parts[0].trim().trim_matches('"').to_string();
                    let mode = parts
                        .get(1)
                        .and_then(|s| s.trim().parse::<u8>().ok())
                        .unwrap_or(0);
                    // Mode 0/2 = read+write (create if missing); mode 1 = read-only
                    // (must exist); mode 3 = write-only. We accept all four.
                    if mode == 1 && !self.filesystem.contains_key(&name) {
                        return self.cme_error(409);
                    }
                    self.filesystem.entry(name.clone()).or_default();
                    let handle = self.next_file_handle;
                    self.next_file_handle = self.next_file_handle.wrapping_add(1).max(1);
                    self.open_files.insert(handle, (name, 0, mode));
                    self.emit(&format!("\r\n+QFOPEN: {}\r\n", handle));
                    return self.ok();
                }
                self.error();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qfread(&mut self, form: &AtForm<'_>, _line: &str, upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QFREAD: <filehandle>[,<length>]\r\n");
                self.ok();
            }
            AtForm::Write(_) => {
                if let Some(arg) = upper.strip_prefix("AT+QFREAD=") {
                    let mut parts = arg.split(',');
                    let handle = parts.next().and_then(|s| s.trim().parse::<u16>().ok());
                    let max_len = parts.next().and_then(|s| s.trim().parse::<usize>().ok());
                    return match handle.and_then(|h| {
                        let entry = self.open_files.get(&h)?.clone();
                        Some((h, entry))
                    }) {
                        Some((h, (name, offset, _))) => {
                            let data = self.filesystem.get(&name).cloned().unwrap_or_default();
                            let avail = data.len().saturating_sub(offset);
                            let take = max_len.unwrap_or(avail).min(avail);
                            let slice = &data[offset..offset + take];
                            let mut payload = format!("\r\nCONNECT {}\r\n", take).into_bytes();
                            payload.extend_from_slice(slice);
                            payload.extend_from_slice(b"\r\nOK\r\n");
                            self.schedule(DELAY_DEFAULT_US, payload);
                            if let Some(entry) = self.open_files.get_mut(&h) {
                                entry.1 = offset + take;
                            }
                        }
                        _ => self.cme_error(409),
                    };
                }
                self.error();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qfwrite(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QFWRITE: <filehandle>[,<length>[,<timeout>]]\r\n");
                self.ok();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qfclose(&mut self, form: &AtForm<'_>, _line: &str, upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QFCLOSE: <filehandle>\r\n");
                self.ok();
            }
            AtForm::Write(_) => {
                if let Some(arg) = upper.strip_prefix("AT+QFCLOSE=") {
                    let handle = arg.trim().parse::<u16>().ok();
                    return match handle {
                        Some(h) if self.open_files.remove(&h).is_some() => self.ok(),
                        _ => self.cme_error(409),
                    };
                }
                self.error();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qfdel(&mut self, form: &AtForm<'_>, line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QFDEL: <filename>\r\n");
                self.ok();
            }
            AtForm::Write(_) => {
                // HAZARD: original check used the case-sensitive `line` prefix.
                if let Some(arg) = line.strip_prefix("AT+QFDEL=") {
                    let name = arg.trim().trim_matches('"');
                    return match self.filesystem.remove(name) {
                        Some(_) => self.ok(),
                        None if name == "*" => {
                            self.filesystem.clear();
                            self.ok()
                        }
                        None => self.cme_error(409),
                    };
                }
                self.error();
            }
            _ => self.error(),
        }
    }
}
