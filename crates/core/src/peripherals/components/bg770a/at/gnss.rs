// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! GNSS AT commands: `+QGPS`, `+QGPSLOC`, `+QGPSCFG`, `+QGPSEND`.

use super::super::*;
use super::AtForm;

impl QuectelBg770a {
    pub(super) fn at_qgps(&mut self, form: &AtForm<'_>, _line: &str, upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QGPS: (1)[,(1-3)[,(0-1000)[,(1-65535)]\r\n");
                self.ok();
            }
            AtForm::Read => {
                self.emit(&format!("\r\n+QGPS: {}\r\n", self.gps_active as u8));
                self.ok();
            }
            AtForm::Write(_) => {
                if let Some(arg) = upper.strip_prefix("AT+QGPS=") {
                    return match arg
                        .split(',')
                        .next()
                        .and_then(|s| s.trim().parse::<u8>().ok())
                    {
                        Some(1) => {
                            self.gps_active = true;
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

    pub(super) fn at_qgpsloc(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit("\r\n+QGPSLOC: (0-5),(0-3600)\r\n");
                self.ok();
            }
            AtForm::Exec | AtForm::Write(_) => {
                if self.gps_active {
                    // Datasheet format: <UTC>,<lat>,<lon>,<HDOP>,<altitude>,
                    // <fix>,<COG>,<spkm>,<spkn>,<date>,<nsat>.
                    self.emit(
                        "\r\n+QGPSLOC: 120000.0,37.7749N,122.4194W,1.0,10.0,3,0.0,0.0,0.0,150626,08\r\n",
                    );
                    self.ok()
                } else {
                    // CME 516: "Not fix now" (Quectel-specific).
                    self.cme_error(516)
                }
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qgpscfg(&mut self, form: &AtForm<'_>, line: &str, _upper: &str) {
        match form {
            AtForm::Test => {
                self.emit(
                    "\r\n+QGPSCFG: \"outport\",(\"none\",\"usbnmea\",\"uartnmea\",\"auxnmea\"),\
(4800,9600,19200,38400,57600,115200,230400,460800,921600)\r\n\
                     +QGPSCFG: \"gnssconfig\",(1)\r\n\
                     +QGPSCFG: \"nmeafmt\",(0,1)\r\n\
                     +QGPSCFG: \"gpsnmeatype\",(0-31)\r\n\
                     +QGPSCFG: \"glonassnmeatype\",(0-3)\r\n\
                     +QGPSCFG: \"nmeasrc\",(0,1)\r\n\
                     +QGPSCFG: \"autogps\",(0,1)\r\n\
                     +QGPSCFG: \"priority\",(0,1)[,(0,1)]\r\n\
                     +QGPSCFG: \"xtrafilesize\",(1,3,7)\r\n\
                     +QGPSCFG: \"xtra_info\"\r\n\
                     +QGPSCFG: \"gpsdop\"\r\n\
                     +QGPSCFG: \"estimation_error\"\r\n\
                     +QGPSCFG: \"xtra_download\",<type>\r\n\
                     +QGPSCFG: \"agnssjamming\",(0-4)[,(2-10),(1-65535)]\r\n\
                     +QGPSCFG: \"agnssjammingurcmode\",(0,1)\r\n\
                     +QGPSCFG: \"test_mode\",<mode>\r\n",
                );
                self.ok();
            }
            AtForm::Write(_) => {
                // HAZARD: original check used the case-sensitive `line` prefix.
                if let Some(args) = line.strip_prefix("AT+QGPSCFG=") {
                    if let Some((key, rest)) = parse_quoted_subkey(args) {
                        let key_lower = key.to_ascii_lowercase();
                        return match (key_lower.as_str(), rest.is_empty()) {
                            ("outport", true) => {
                                self.emit(&format!(
                                    "\r\n+QGPSCFG: \"outport\",\"{}\",{}\r\n",
                                    self.qgps_outport, self.qgps_outport_baud
                                ));
                                self.ok()
                            }
                            ("outport", false) => {
                                let port = rest[0].trim().trim_matches('"').to_string();
                                let baud = rest
                                    .get(1)
                                    .and_then(|s| s.trim().parse::<u32>().ok())
                                    .unwrap_or(115200);
                                self.qgps_outport = port;
                                self.qgps_outport_baud = baud;
                                self.ok()
                            }
                            ("autogps", true) => {
                                self.emit(&format!(
                                    "\r\n+QGPSCFG: \"autogps\",{}\r\n",
                                    self.qgps_autogps
                                ));
                                self.ok()
                            }
                            ("autogps", false) => {
                                if let Ok(v) = rest[0].trim().parse::<u8>() {
                                    self.qgps_autogps = v.min(1);
                                }
                                self.ok()
                            }
                            ("nmeasrc", true) => {
                                self.emit(&format!(
                                    "\r\n+QGPSCFG: \"nmeasrc\",{}\r\n",
                                    self.qgps_nmeasrc
                                ));
                                self.ok()
                            }
                            ("nmeasrc", false) => {
                                if let Ok(v) = rest[0].trim().parse::<u8>() {
                                    self.qgps_nmeasrc = v.min(1);
                                }
                                self.ok()
                            }
                            ("gnssconfig", true) => {
                                self.emit(&format!(
                                    "\r\n+QGPSCFG: \"gnssconfig\",{}\r\n",
                                    self.qgps_gnssconfig
                                ));
                                self.ok()
                            }
                            _ => self.ok(),
                        };
                    }
                    return self.ok();
                }
                self.error();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qgpsend(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Exec => {
                if self.gps_active {
                    self.gps_active = false;
                    self.ok()
                } else {
                    // Real HW quirk: QGPSEND always emits the verbose `+CME ERROR:
                    // 505` (GPS not active) form, even when CMEE=0 — bypassing the
                    // usual CMEE-mapping that would otherwise collapse this to a
                    // bare `ERROR`. Captured directly from the bench.
                    self.emit("\r\n+CME ERROR: 505\r\n");
                }
            }
            _ => self.error(),
        }
    }
}
