// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Identity AT commands: `ATI`, `AT+CGMI`/`+GMI`, `+CGMM`/`+GMM`,
//! `+CGMR`/`+GMR`, `+CGSN`, `+CIMI`, `+QCCID`.

use super::super::*;
use super::AtForm;

impl QuectelBg770a {
    pub(super) fn at_i(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Exec => {
                self.emit(&format!(
                    "\r\n{}\r\n{}\r\nRevision: {}\r\n",
                    ID_MANUFACTURER, ID_MODEL, ID_FIRMWARE
                ));
                self.ok();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_i1(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        self.at_i(form, _line, _upper);
    }

    pub(super) fn at_cgmi(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Exec => {
                self.emit(&format!("\r\n{}\r\n", ID_MANUFACTURER));
                self.ok();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_gmi(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        self.at_cgmi(form, _line, _upper);
    }

    pub(super) fn at_cgmm(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Exec => {
                self.emit(&format!("\r\n{}\r\n", ID_MODEL));
                self.ok();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_gmm(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        self.at_cgmm(form, _line, _upper);
    }

    pub(super) fn at_cgmr(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Exec => {
                self.emit(&format!("\r\n{}\r\n", ID_FIRMWARE));
                self.ok();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_gmr(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        self.at_cgmr(form, _line, _upper);
    }

    pub(super) fn at_cgsn(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Exec => {
                self.emit(&format!("\r\n{}\r\n", FAKE_IMEI));
                self.ok();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_cimi(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Exec => {
                self.emit(&format!("\r\n{}\r\n", FAKE_IMSI));
                self.ok();
            }
            _ => self.error(),
        }
    }

    pub(super) fn at_qccid(&mut self, form: &AtForm<'_>, _line: &str, _upper: &str) {
        match form {
            AtForm::Exec => {
                self.emit(&format!("\r\n+QCCID: {}\r\n", FAKE_ICCID));
                self.ok();
            }
            _ => self.error(),
        }
    }
}
