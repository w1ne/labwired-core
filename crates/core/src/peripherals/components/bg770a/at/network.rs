// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Signal quality, registration, operator selection, packet-domain / PDP and
//! network-info AT commands: `+CSQ`, `+QCSQ`, `+CEREG`, `+CREG`, `+COPS`,
//! `+CGATT`, `+CGACT`, `+CGPADDR`, `+CGDCONT`, `+QICSGP`, `+QIACT`,
//! `+QIDEACT`, `+QIGETERROR`, `+QNWINFO`, `+QENG`, `+CEINFO`.

use super::super::*;
use super::AtForm;

impl QuectelBg770a {
    pub(super) fn at_csq(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+CSQ: (0-31,99),(0-7,99)\r\n");
                self.ok();
            }
            AtForm::Exec => {
                let (rssi, ber) = self.effective_csq();
                self.emit(&format!("\r\n+CSQ: {},{}\r\n", rssi, ber));
                self.ok();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qcsq(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Exec => {
                let (csq, _) = self.effective_csq();
                if csq >= 99 {
                    self.emit("\r\n+QCSQ: \"NOSERVICE\"\r\n");
                } else {
                    // Derived RSRP/RSRQ values for the populated CSQ; -113 dBm
                    // baseline + 2 dB per CSQ step is the standard mapping.
                    let rssi_dbm = -113 + 2 * csq as i16;
                    let rsrp = rssi_dbm - 18; // approximate eMTC offset
                    self.emit(&format!(
                        "\r\n+QCSQ: \"eMTC\",{},{},{},{}\r\n",
                        rssi_dbm, rsrp, 200, -10
                    ));
                }
                self.ok();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_cereg(&mut self, form: &AtForm<'_>, _line: &str, upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+CEREG: (0-2,4)\r\n");
                self.ok();
            }
            AtForm::Read => {
                self.emit(&format!(
                    "\r\n+CEREG: {},{}\r\n",
                    self.cereg_n, self.cereg_stat
                ));
                self.ok();
            }
            AtForm::Write(_) => {
                if let Some(arg) = upper.strip_prefix("AT+CEREG=") {
                    return match arg.trim().parse::<u8>() {
                        Ok(v @ (0..=2 | 4)) => {
                            self.cereg_n = v;
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

    pub(super) fn at_creg(&mut self, form: &AtForm<'_>, _line: &str, upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+CREG: (0-2)\r\n");
                self.ok();
            }
            AtForm::Read => {
                self.emit(&format!(
                    "\r\n+CREG: {},{}\r\n",
                    self.creg_n, self.cereg_stat
                ));
                self.ok();
            }
            AtForm::Write(_) => {
                if let Some(arg) = upper.strip_prefix("AT+CREG=") {
                    return match arg.trim().parse::<u8>() {
                        Ok(v @ 0..=2) => {
                            self.creg_n = v;
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

    pub(super) fn at_cops(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Read => {
                self.emit("\r\n+COPS: 0\r\n");
                self.ok();
            }
            AtForm::Test => {
                // Real-hardware quirk: unattached → +CME ERROR: 515.
                self.cme_error(515);
            }
            AtForm::Write(_) => {
                self.current_delay_us = DELAY_COPS_WRITE_US;
                self.ok();
            }
            AtForm::Exec => self.error(),
        }
    }

    pub(super) fn at_cgatt(&mut self, form: &AtForm<'_>, _line: &str, upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+CGATT: (0-1)\r\n");
                self.ok();
            }
            AtForm::Read => {
                self.emit(&format!("\r\n+CGATT: {}\r\n", self.cgatt));
                self.ok();
            }
            AtForm::Write(_) => {
                if let Some(arg) = upper.strip_prefix("AT+CGATT=") {
                    return match arg.trim().parse::<u8>() {
                        Ok(v @ 0..=1) => {
                            self.cgatt = v;
                            self.current_delay_us = DELAY_CGATT_WRITE_US;
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

    pub(super) fn at_cgact(&mut self, form: &AtForm<'_>, _line: &str, upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+CGACT: (0-1)\r\n");
                self.ok();
            }
            AtForm::Read => {
                self.emit(&format!("\r\n+CGACT: 1,{}\r\n", self.cgact_cid1));
                self.ok();
            }
            AtForm::Write(_) => {
                if let Some(arg) = upper.strip_prefix("AT+CGACT=") {
                    let mut parts = arg.split(',');
                    let state = parts.next().and_then(|s| s.trim().parse::<u8>().ok());
                    let cid = parts.next().and_then(|s| s.trim().parse::<u8>().ok());
                    return match (state, cid) {
                        (Some(s @ 0..=1), Some(1)) => {
                            self.cgact_cid1 = s;
                            self.current_delay_us = DELAY_CGACT_WRITE_US;
                            self.ok()
                        }
                        // Activation requires attach; deactivation always allowed.
                        _ => self.error(),
                    };
                }
                self.error();
            }
            AtForm::Exec => self.error(),
        }
    }

    pub(super) fn at_cgpaddr(&mut self, form: &AtForm<'_>, _line: &str, upper: &str) {
        match form {
            // CGPADDR test form narrows to defined cids on real hardware — we
            // only define cid 1, so the chip reports `(1)`, not the manual's
            // generic `(1-15)`.
            AtForm::Test => {
                self.emit("\r\n+CGPADDR: (1)\r\n");
                self.ok();
            }
            AtForm::Write(_) => {
                if let Some(arg) = upper.strip_prefix("AT+CGPADDR=") {
                    return match arg.trim().parse::<u8>() {
                        Ok(1) => {
                            // When the context isn't activated, real HW omits the
                            // address field entirely — just `+CGPADDR: 1`.
                            if self.cgact_cid1 == 1 {
                                self.emit("\r\n+CGPADDR: 1,10.0.0.2\r\n");
                            } else {
                                self.emit("\r\n+CGPADDR: 1\r\n");
                            }
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

    pub(super) fn at_cgdcont(&mut self, form: &AtForm<'_>, line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit(
                    "\r\n+CGDCONT: (1-15),\"IP\",,,(0),(0),(0)\r\n\
                     +CGDCONT: (1-15),\"IPV6\",,,(0),(0),(0)\r\n\
                     +CGDCONT: (1-15),\"IPV4V6\",,,(0),(0),(0)\r\n\
                     +CGDCONT: (1-15),\"Non-IP\",,,(0),(0),(0)\r\n",
                );
                self.ok();
            }
            AtForm::Read => {
                self.emit(&format!(
                    "\r\n+CGDCONT: 1,\"{}\",\"{}\",\"0.0.0.0\",0,0,0\r\n",
                    self.pdp_type, self.pdp_apn
                ));
                self.ok();
            }
            AtForm::Write(_) => {
                // HAZARD: the original check was a case-sensitive
                // `line.strip_prefix("AT+CGDCONT=")` (not `upper`), so a
                // lower-case command name fell through to ERROR. Reproduce it.
                if let Some(arg) = line.strip_prefix("AT+CGDCONT=") {
                    // Datasheet: AT+CGDCONT=<cid>[,<PDP_type>[,<APN>...]]
                    // Strings are double-quoted; we only model cid=1 with PDP_type+APN.
                    let mut parts = arg.splitn(3, ',');
                    let cid = parts.next().and_then(|s| s.trim().parse::<u8>().ok());
                    let pdp_type = parts.next().map(|s| s.trim().trim_matches('"'));
                    let apn = parts.next().and_then(|rest| {
                        // APN is the third field; strip the surrounding quotes and stop
                        // at the first comma not inside a quote.
                        let mut chars = rest.chars().peekable();
                        if chars.next()? != '"' {
                            return None;
                        }
                        let mut out = String::new();
                        for c in chars {
                            if c == '"' {
                                return Some(out);
                            }
                            out.push(c);
                        }
                        None
                    });
                    return match (cid, pdp_type, apn) {
                        (Some(1), Some(pt), Some(apn))
                            if matches!(pt, "IP" | "IPV6" | "IPV4V6" | "Non-IP") =>
                        {
                            self.pdp_type = pt.to_string();
                            self.pdp_apn = apn;
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

    pub(super) fn at_qicsgp(&mut self, form: &AtForm<'_>, _line: &str, upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QICSGP: (1-5),(1-3),<APN>,<username>,<password>,(0-2)\r\n");
                self.ok();
            }
            AtForm::Write(_) => {
                if let Some(arg) = upper.strip_prefix("AT+QICSGP=") {
                    // AT+QICSGP=<cid>,<ctx_type>,"<apn>","<user>","<pwd>",<auth>
                    let first = arg
                        .split(',')
                        .next()
                        .and_then(|s| s.trim().parse::<u8>().ok());
                    return match first {
                        Some(1..=5) => self.ok(),
                        _ => self.error(),
                    };
                }
                self.error();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qiact(&mut self, form: &AtForm<'_>, _line: &str, upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QIACT: (1-5)\r\n");
                self.ok();
            }
            AtForm::Read => {
                // Real HW: returns no `+QIACT:` lines when no context is active —
                // just bare OK.
                if self.qiact_cid1 == 1 {
                    self.emit("\r\n+QIACT: 1,1,1,\"10.0.0.2\"\r\n");
                }
                self.ok();
            }
            AtForm::Write(_) => {
                if let Some(arg) = upper.strip_prefix("AT+QIACT=") {
                    return match arg.trim().parse::<u8>() {
                        Ok(1) => {
                            self.qiact_cid1 = 1;
                            self.current_delay_us = DELAY_QIACT_US;
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

    pub(super) fn at_at_qideact(&mut self, form: &AtForm<'_>, _line: &str, upper: &str) {
        match form {
            AtForm::Write(_) => {
                if let Some(arg) = upper.strip_prefix("AT+QIDEACT=") {
                    return match arg.trim().parse::<u8>() {
                        Ok(1) => {
                            self.qiact_cid1 = 0;
                            self.current_delay_us = DELAY_QIACT_US;
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

    pub(super) fn at_qigeterror(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Exec => {
                self.emit("\r\n+QIGETERROR: 0,operate successfully\r\n");
                self.ok();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qnwinfo(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Exec => {
                // Real HW returns `+QNWINFO: "NBIoT","21670","LTE BAND 1",0` even
                // when not attached — the cached "last seen" cell info.
                self.emit("\r\n+QNWINFO: \"NBIoT\",\"21670\",\"LTE BAND 1\",0\r\n");
                self.ok();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qeng(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QENG: (\"servingcell\",\"neighbourcell\")\r\n");
                self.ok();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_ceinfo(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+CEINFO: (0)\r\n");
                self.ok();
            }
            _ => self.error(),
        }
    }
}
