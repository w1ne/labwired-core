// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! HTTP AT commands: `+QHTTPCFG`, `+QHTTPURL`, `+QHTTPGET`, `+QHTTPPOST`,
//! `+QHTTPREAD`.
//!
//! A sync command takes the modem into a CONNECT-prompt mode; firmware
//! streams the URL or POST body bytes; modem then returns OK and (for
//! GET/POST) emits an async `+QHTTPGET/POST: <err>,<code>,<len>` URC.

use super::super::*;
use super::AtForm;

impl QuectelBg770a {
    pub(super) fn at_qhttpcfg(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit(
                    "\r\n+QHTTPCFG: \"contextid\",(1-5)\r\n\
                     +QHTTPCFG: \"requestheader\",(0,1)\r\n\
                     +QHTTPCFG: \"responseheader\",(0,1)\r\n\
                     +QHTTPCFG: \"sslctxid\",(0-5)\r\n\
                     +QHTTPCFG: \"contenttype\",(0-5)\r\n\
                     +QHTTPCFG: \"auth\",(\"username:password\")\r\n\
                     +QHTTPCFG: \"custom_header\",(\"custom_value\")\r\n",
                );
                self.ok();
            }
            AtForm::Write(_) => {
                // Accept any sub-key (contextid, requestheader, sslctxid, etc.).
                self.ok();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qhttpurl(&mut self, form: &AtForm<'_>, _line: &str, upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QHTTPURL: (1-700),(1-65535)\r\n");
                self.ok();
            }
            AtForm::Write(_) => {
                // QHTTPURL=<len>,<timeout> — modem emits CONNECT, firmware streams
                // exactly <len> bytes of the URL, then modem replies with OK.
                if let Some(arg) = upper.strip_prefix("AT+QHTTPURL=") {
                    let url_len = arg
                        .split(',')
                        .next()
                        .and_then(|s| s.trim().parse::<usize>().ok());
                    return match url_len {
                        Some(n @ 1..=700) => {
                            self.emit("\r\nCONNECT\r\n");
                            self.awaiting_http_data = Some((HttpPromptKind::Url, n));
                            self.http_data_buf.clear();
                        }
                        _ => self.error(),
                    };
                }
                self.error();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qhttpget(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QHTTPGET: (1-65535),(1-2048),(1-65535)\r\n");
                self.ok();
            }
            AtForm::Write(_) => {
                // QHTTPGET=<rsp_timeout>[,...] — sync OK, then async
                //   +QHTTPGET: <err>,<httprspcode>,<content_length>
                self.ok();
                let urc = format!(
                    "\r\n+QHTTPGET: 0,{},{}\r\n",
                    self.http_response_code,
                    self.http_response_body.len()
                );
                self.deferred_urcs
                    .push((URC_DELAY_QIDNS_US, urc.into_bytes()));
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qhttppost(&mut self, form: &AtForm<'_>, _line: &str, upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QHTTPPOST: (1-1024000),(1-65535),(1-65535)\r\n");
                self.ok();
            }
            AtForm::Write(_) => {
                // QHTTPPOST=<bodyLen>,<rspTimeout>,<reqDataTimeout> — CONNECT prompt,
                // firmware streams body, modem returns OK + async URC.
                if let Some(arg) = upper.strip_prefix("AT+QHTTPPOST=") {
                    let body_len = arg
                        .split(',')
                        .next()
                        .and_then(|s| s.trim().parse::<usize>().ok());
                    return match body_len {
                        Some(n @ 1..=1_024_000) => {
                            self.emit("\r\nCONNECT\r\n");
                            self.awaiting_http_data = Some((HttpPromptKind::PostBody, n));
                            self.http_data_buf.clear();
                        }
                        _ => self.error(),
                    };
                }
                self.error();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qhttpread(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QHTTPREAD: (1-65535)\r\n");
                self.ok();
            }
            AtForm::Write(_) => {
                // QHTTPREAD=<waittime> — modem emits CONNECT, dumps the response
                // body, then `\r\nOK\r\n` and `+QHTTPREAD: 0` URC.
                let mut payload = b"\r\nCONNECT\r\n".to_vec();
                payload.extend_from_slice(&self.http_response_body);
                payload.extend_from_slice(b"\r\nOK\r\n\r\n+QHTTPREAD: 0\r\n");
                // Replace the auto-OK with this manual emission, so we don't
                // double-up. The handler exits without setting respond_buf and
                // schedules the bytes itself.
                self.schedule(DELAY_DEFAULT_US, payload);
            }
            _ => self.error(),
        }
    }
}
