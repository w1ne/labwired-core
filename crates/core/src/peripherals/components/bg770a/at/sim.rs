// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! SIM/PIN, error-reporting and functionality AT commands:
//! `+CPIN`, `+CMEE`, `+CFUN`.

use super::super::*;
use super::AtForm;

impl QuectelBg770a {
    pub(super) fn at_cpin(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Test => self.ok(),
            AtForm::Read => {
                self.emit("\r\n+CPIN: READY\r\n");
                self.ok();
            }
            AtForm::Write(_) => {
                // Datasheet "Maximum Response Time: 5 s" for the write form. Even
                // when we immediately reject it (SIM already READY), the chip
                // still takes the time to validate — model the delay.
                self.current_delay_us = DELAY_CPIN_WRITE_US;
                self.cme_error(3);
            }
            AtForm::Exec => self.error(),
        }
    }

    pub(super) fn at_cmee(&mut self, form: &AtForm<'_>, _line: &str, upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+CMEE: (0-2)\r\n");
                self.ok();
            }
            AtForm::Read => {
                self.emit(&format!("\r\n+CMEE: {}\r\n", self.cmee_mode));
                self.ok();
            }
            AtForm::Write(_) => {
                if let Some(arg) = upper.strip_prefix("AT+CMEE=") {
                    return match arg.trim().parse::<u8>() {
                        Ok(v) if v <= 2 => {
                            self.cmee_mode = v;
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

    pub(super) fn at_cfun(&mut self, form: &AtForm<'_>, _line: &str, upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+CFUN: (0-1,4),(0-1)\r\n");
                self.ok();
            }
            AtForm::Read => {
                self.emit(&format!("\r\n+CFUN: {}\r\n", self.cfun));
                self.ok();
            }
            AtForm::Write(_) => {
                if let Some(arg) = upper.strip_prefix("AT+CFUN=") {
                    let fun = arg
                        .split(',')
                        .next()
                        .and_then(|s| s.trim().parse::<u8>().ok());
                    return match fun {
                        Some(v @ (0 | 1 | 4)) => {
                            let prev = self.cfun;
                            self.cfun = v;
                            self.current_delay_us = DELAY_CFUN_WRITE_US;
                            self.ok();
                            // 0 → 1 transition replays the SIM-init URCs the chip
                            // emits when the radio comes back up. They must enqueue
                            // *after* the OK, so defer the actual scheduling to
                            // on_tx_byte which runs after the response is queued.
                            if prev == 0 && v == 1 {
                                self.pending_cfun_resume_urcs = true;
                            }
                        }
                        _ => self.error(),
                    };
                }
                self.error();
            }
            AtForm::Exec => self.error(),
        }
    }
}
