// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Everything not covered by a dedicated family: SMS, power-save, clock/NTP,
//! FOTA/version, misc utility, the AT% Sequans and AT+VZ Verizon extension
//! surfaces, and power-off.

use super::super::*;
use super::AtForm;

impl QuectelBg770a {
    // ----- SMS -------------------------------------------------------------

    pub(super) fn at_cmgf(&mut self, form: &AtForm<'_>, _line: &str, upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+CMGF: (0,1)\r\n");
                self.ok();
            }
            AtForm::Read => {
                self.emit(&format!("\r\n+CMGF: {}\r\n", self.cmgf));
                self.ok();
            }
            AtForm::Write(_) => {
                if let Some(arg) = upper.strip_prefix("AT+CMGF=") {
                    return match arg.trim().parse::<u8>() {
                        Ok(v @ 0..=1) => {
                            self.cmgf = v;
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

    pub(super) fn at_cnmi(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+CNMI: (1-2),(0-2),(0,2),(0-2),(0-1)\r\n");
                self.ok();
            }
            AtForm::Read => {
                self.emit("\r\n+CNMI: 2,1,0,0,0\r\n");
                self.ok();
            }
            AtForm::Write(_) => {
                // Accept any args; we don't surface incoming-SMS URCs anyway.
                self.ok();
            }
            AtForm::Exec => self.error(),
        }
    }

    pub(super) fn at_cmgs(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Test => self.ok(),
            AtForm::Write(_) => {
                // CMGS write — text mode (CMGF=1): AT+CMGS="number" → `> ` prompt →
                // body + 0x1A → `+CMGS: <mr>` + OK. PDU mode (CMGF=0) uses a length
                // instead of a quoted number; same prompt behaviour.
                self.emit("\r\n> ");
                self.awaiting_cmgs_payload = true;
                self.cmgs_payload_buf.clear();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_cmgr(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Test => self.ok(),
            _ => self.error(),
        }
    }

    pub(super) fn at_cmgl(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+CMGL: (0-4)\r\n");
                self.ok();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_cmgd(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+CMGD: (1-50),(0-4)\r\n");
                self.ok();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_cscs(&mut self, form: &AtForm<'_>, line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+CSCS: (\"IRA\",\"GSM\",\"UCS2\")\r\n");
                self.ok();
            }
            AtForm::Read => {
                self.emit(&format!("\r\n+CSCS: \"{}\"\r\n", self.cscs));
                self.ok();
            }
            AtForm::Write(_) => {
                // HAZARD: original check used the case-sensitive `line` prefix.
                if let Some(arg) = line.strip_prefix("AT+CSCS=") {
                    let val = arg.trim().trim_matches('"');
                    return match val {
                        "GSM" | "IRA" | "UCS2" => {
                            self.cscs = val.to_string();
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

    pub(super) fn at_csca(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Test => self.ok(),
            AtForm::Read => {
                self.emit("\r\n+CSCA: \"+0000000000000\",145\r\n");
                self.ok();
            }
            _ => self.error(),
        }
    }

    // ----- Power save ------------------------------------------------------

    pub(super) fn at_qsclk(&mut self, form: &AtForm<'_>, _line: &str, upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QSCLK: (0-2)\r\n");
                self.ok();
            }
            AtForm::Read => {
                self.emit(&format!("\r\n+QSCLK: {}\r\n", self.qsclk));
                self.ok();
            }
            AtForm::Write(_) => {
                if let Some(arg) = upper.strip_prefix("AT+QSCLK=") {
                    return match arg.trim().parse::<u8>() {
                        Ok(v @ 0..=2) => {
                            self.qsclk = v;
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

    pub(super) fn at_cpsms(&mut self, form: &AtForm<'_>, _line: &str, upper: &str) {
        match form {
            AtForm::Test => {
                self.emit(
                    "\r\n+CPSMS: (0-2),(\"00000000\"-\"10111111\"),(\"00000000\"-\"11111111\"),\
(\"00000000\"-\"10111111\"),(\"00000000\"-\"11111111\")\r\n",
                );
                self.ok();
            }
            AtForm::Read => {
                self.emit(&format!(
                    "\r\n+CPSMS: {},,,\"00101100\",\"00001010\"\r\n",
                    self.cpsms_mode
                ));
                self.ok();
            }
            AtForm::Write(_) => {
                if let Some(arg) = upper.strip_prefix("AT+CPSMS=") {
                    let first = arg
                        .split(',')
                        .next()
                        .and_then(|s| s.trim().parse::<u8>().ok());
                    return match first {
                        Some(v @ 0..=2) => {
                            self.cpsms_mode = v;
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

    pub(super) fn at_cedrxs(&mut self, form: &AtForm<'_>, _line: &str, upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+CEDRXS: (0-3),(4,5),(\"0000\"-\"1111\")\r\n");
                self.ok();
            }
            AtForm::Read => {
                self.emit(&format!("\r\n+CEDRXS: {}\r\n", self.cedrxs_mode));
                self.ok();
            }
            AtForm::Write(_) => {
                if let Some(arg) = upper.strip_prefix("AT+CEDRXS=") {
                    let first = arg
                        .split(',')
                        .next()
                        .and_then(|s| s.trim().parse::<u8>().ok());
                    return match first {
                        Some(v @ 0..=3) => {
                            self.cedrxs_mode = v;
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

    pub(super) fn at_qpsmcfg(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QPSMCFG: (20-4294967295),(0-15)\r\n");
                self.ok();
            }
            AtForm::Write(_) => {
                // Accept the two-arg write form `<threshold>,<version>`; we don't
                // model PSM timing internally yet.
                self.ok();
            }
            _ => self.error(),
        }
    }

    // ----- Clock / NTP -----------------------------------------------------

    pub(super) fn at_cclk(&mut self, form: &AtForm<'_>, line: &str, _upper: &str) {
        match form {
            AtForm::Test => self.ok(),
            AtForm::Read => {
                self.emit(&format!("\r\n+CCLK: \"{}\"\r\n", self.cclk));
                self.ok();
            }
            AtForm::Write(_) => {
                // HAZARD: original check used the case-sensitive `line` prefix.
                if let Some(arg) = line.strip_prefix("AT+CCLK=") {
                    let val = arg.trim().trim_matches('"').to_string();
                    if val.len() >= 17 {
                        self.cclk = val;
                        return self.ok();
                    }
                    return self.error();
                }
                self.error();
            }
            AtForm::Exec => self.error(),
        }
    }

    pub(super) fn at_qlts(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QLTS: (0-2)\r\n");
                self.ok();
            }
            AtForm::Exec | AtForm::Write(_) => {
                // Returns network-derived local time. We reuse the CCLK string.
                self.emit(&format!("\r\n+QLTS: \"{}\",0\r\n", self.cclk));
                self.ok();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qntp(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QNTP: (1-5),<server>,(1-65535),(0,1)\r\n");
                self.ok();
            }
            AtForm::Write(_) => {
                self.ok();
                // Async URC: success result with the current simulated clock.
                let urc = format!("\r\n+QNTP: 0,\"{}\"\r\n", self.cclk);
                self.deferred_urcs
                    .push((URC_DELAY_QIDNS_US, urc.into_bytes()));
            }
            _ => self.error(),
        }
    }

    // ----- FOTA / version --------------------------------------------------

    pub(super) fn at_qfotadl(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Test => self.ok(),
            _ => self.error(),
        }
    }

    pub(super) fn at_qktfota(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Test => self.ok(),
            _ => self.error(),
        }
    }

    pub(super) fn at_qhvn(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QHVN: <hvn>\r\n");
                self.ok();
            }
            AtForm::Exec | AtForm::Read => {
                self.emit("\r\n+QHVN: \"BG770AGLAAR01A05_01.001.01.001\"\r\n");
                self.ok();
            }
            AtForm::Write(_) => self.error(),
        }
    }

    // ----- Misc utility ----------------------------------------------------

    pub(super) fn at_qping(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QPING: (1-5),<host>,(1-255),(1-10)\r\n");
                self.ok();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qlbs(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Test => self.ok(),
            _ => self.error(),
        }
    }

    pub(super) fn at_qlbscfg(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit(
                    "\r\n+QLBSCFG: \"asynch\",(0,1)\r\n\
                     +QLBSCFG: \"timeout\",(10-120)\r\n\
                     +QLBSCFG: \"server\",<server_name>\r\n\
                     +QLBSCFG: \"token\",<token_value>\r\n\
                     +QLBSCFG: \"timeupdate\",(0,1)\r\n\
                     +QLBSCFG: \"withtime\",(0,1)\r\n\
                     +QLBSCFG: \"latorder\",(0,1)\r\n\
                     +QLBSCFG: \"scanband\",(0,1),<scan_band>\r\n\
                     +QLBSCFG: \"singlecell\",(0,1)\r\n",
                );
                self.ok();
            }
            AtForm::Write(_) => self.ok(),
            _ => self.error(),
        }
    }

    // ----- AT% Sequans extensions ------------------------------------------

    pub(super) fn at_ratact(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Read => {
                self.emit("\r\n%RATACT: \"NBIOT\",1,0\r\n");
                self.ok_compact();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_ratsw(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Read => {
                self.emit("\r\n%RATSW: 2,1\r\n");
                self.ok();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_mqttcmd(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                // The real chip drops the leading `\r\n` for `%MQTTCMD` and uses
                // the compact OK form. Mirror exactly.
                self.respond_buf.extend_from_slice(
                    b"\n%MQTTCMD: (\"CONNECT\",\"DISCONNECT\",\"SUBSCRIBE\",\"UNSUBSCRIBE\",\"PUBLISH\"),(0-5)\r\n",
                );
                self.ok_compact();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_certcmd(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                // Note the trailing space before the final \r\n — captured from
                // hardware verbatim, a Sequans-firmware quirk.
                self.emit(
                    "\r\n%CERTCMD: (\"READ\",\"WRITE\",\"DELETE\",\"DIR\",\"COPY\"),(0,1,2,3) \r\n",
                );
                self.ok_compact();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_meas(&mut self, form: &AtForm<'_>, _line: &str, upper: &str) {
        match form {
            AtForm::Write(_) if upper == "AT%MEAS=\"8\"" => {
                self.emit(
                    "\r\n%MEAS: Signal Quality: RSRP = N/A, RSRQ = N/A, SINR = N/A, RSSI = N/A\r\n",
                );
                self.ok();
            }
            AtForm::Read => {
                self.emit("\r\n+CME ERROR: operation not allowed\r\n");
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_pdnstat(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Read => self.ok(),
            _ => self.error(),
        }
    }

    pub(super) fn at_scan(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Test => self.ok(),
            _ => self.error(),
        }
    }

    pub(super) fn at_pcoinfo(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Read => self.ok(),
            _ => self.error(),
        }
    }

    pub(super) fn at_pdnset(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Test => self.ok(),
            _ => self.error(),
        }
    }

    pub(super) fn at_statev(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Read => self.emit("\r\n+CME ERROR: operation not allowed\r\n"),
            _ => self.error(),
        }
    }

    pub(super) fn at_pconi(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Read => self.emit("\r\n+CME ERROR: operation not allowed\r\n"),
            _ => self.error(),
        }
    }

    pub(super) fn at_scancfg(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Read => self.emit("\r\n+CME ERROR: operation not allowed\r\n"),
            _ => self.error(),
        }
    }

    pub(super) fn at_ccid(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Read => self.emit("\r\n+CME ERROR: operation not allowed\r\n"),
            _ => self.error(),
        }
    }

    pub(super) fn at_status(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Exec => self.emit("\r\n+CME ERROR: Incorrect parameters\r\n"),
            _ => self.error(),
        }
    }

    pub(super) fn at_pdnrdp(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Read => self.error(),
            _ => self.error(),
        }
    }

    // ----- AT+VZ Verizon extensions ----------------------------------------

    pub(super) fn at_vzwapne(&mut self, _form: &AtForm<'_>, _line: &str, _upper: &str) {
        self.error();
    }

    // ----- Power -----------------------------------------------------------

    pub(super) fn at_at_qpowd(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        let fire = match form {
            AtForm::Exec => true,
            AtForm::Write(arg) => *arg == "0" || *arg == "1",
            _ => false,
        };
        if fire {
            // Manual: emits OK, then `POWERED DOWN` URC ~600-800 ms later,
            // then the modem is silent until PWRKEY toggle.
            self.ok();
            self.schedule(700_000, b"\r\nPOWERED DOWN\r\n".to_vec());
            self.powered_off = true;
        } else {
            self.error();
        }
    }
}
