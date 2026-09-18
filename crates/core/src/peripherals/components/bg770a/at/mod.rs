// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Table-driven AT dispatch for the Quectel BG770A model.
//!
//! `handle_line` used to be one chain of `if upper == "AT+X=?"` / `if let
//! Some(arg) = upper.strip_prefix("AT+X=")` blocks for ~100 commands. This
//! module replaces the name+form matching with [`parse_at`] and one sorted
//! handler table ([`AT_HANDLERS`]); each handler owns the former bodies for
//! one command, grouped by family in the sibling submodules.
//!
//! The table maps the command token (e.g. `"+CPIN"`, `"%RATACT"`, `"I"`) to a
//! function that receives the parsed [`AtForm`] plus the original `line` and
//! uppercased `upper` byte-for-byte, so handlers that matched case-sensitively
//! against `line` keep doing so and handlers that matched `upper` keep doing so.

use super::QuectelBg770a;

mod files;
mod gnss;
mod http;
mod identity;
mod misc;
mod mqtt;
mod network;
mod sim;
mod sockets;

/// One form of an AT request. `Write` carries the raw argument text after the
/// first `=` (case preserved from whatever string [`parse_at`] was handed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtForm<'a> {
    /// `AT+X=?` — the "test" form.
    Test,
    /// `AT+X?` — the "read" form.
    Read,
    /// `AT+X=<args>` — the "write" form; `<args>` may be empty.
    Write(&'a str),
    /// `AT+X` — the "exec"/action form.
    Exec,
}

/// A parsed AT request: the command token and its form. `name` is `""` for the
/// bare `AT` probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtRequest<'a> {
    pub name: &'a str,
    pub form: AtForm<'a>,
}

/// Parse an (already trimmed, ideally uppercased) AT line into a command token
/// and its form.
///
/// Splits at the FIRST `=` or `?` after the name:
///   * `AT+CPIN=?`   → `+CPIN`, `Test`
///   * `AT+CPIN?`    → `+CPIN`, `Read`
///   * `AT+CPIN=1234`→ `+CPIN`, `Write("1234")`
///   * `AT+CSQ`      → `+CSQ`, `Exec`
///   * `AT`          → ``, `Exec`
///
/// A `?` that is not the final character (and not preceded by `=`) does NOT
/// form a request the modem recognises, so it yields `None` — mirroring the
/// old exact-string matching, where `AT+CSQ?x` fell through to `ERROR`.
pub fn parse_at(upper: &str) -> Option<AtRequest<'_>> {
    if !upper.get(..2).is_some_and(|p| p.eq_ignore_ascii_case("AT")) {
        return None;
    }
    let rest = &upper[2..];
    let bytes = rest.as_bytes();
    let mut delim: Option<(usize, u8)> = None;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'=' || b == b'?' {
            delim = Some((i, b));
            break;
        }
    }
    match delim {
        None => Some(AtRequest {
            name: rest,
            form: AtForm::Exec,
        }),
        Some((i, b'=')) => {
            let name = &rest[..i];
            let after = &rest[i + 1..];
            if after == "?" {
                Some(AtRequest {
                    name,
                    form: AtForm::Test,
                })
            } else {
                Some(AtRequest {
                    name,
                    form: AtForm::Write(after),
                })
            }
        }
        Some((i, b'?')) => {
            // Recognised only when the `?` is the whole trailing marker.
            if i == bytes.len() - 1 {
                Some(AtRequest {
                    name: &rest[..i],
                    form: AtForm::Read,
                })
            } else {
                None
            }
        }
        Some(_) => None,
    }
}

/// Signature shared by every entry in [`AT_HANDLERS`]. In addition to the
/// parsed `form`, a handler receives the original `line` and its uppercase
/// `upper` so bodies that relied on exact `line`/`upper` slicing stay
/// byte-identical.
pub type AtHandler = fn(&mut QuectelBg770a, &AtForm<'_>, &str, &str);

/// Sorted (lexicographic by token) command dispatch table. Sortedness is
/// checked by `at_handlers_table_is_sorted` and looked up with a binary search.
pub static AT_HANDLERS: &[(&str, AtHandler)] = &[
    ("%CCID", QuectelBg770a::at_ccid),
    ("%CERTCMD", QuectelBg770a::at_certcmd),
    ("%MEAS", QuectelBg770a::at_meas),
    ("%MQTTCMD", QuectelBg770a::at_mqttcmd),
    ("%PCOINFO", QuectelBg770a::at_pcoinfo),
    ("%PCONI", QuectelBg770a::at_pconi),
    ("%PDNRDP", QuectelBg770a::at_pdnrdp),
    ("%PDNSET", QuectelBg770a::at_pdnset),
    ("%PDNSTAT", QuectelBg770a::at_pdnstat),
    ("%RATACT", QuectelBg770a::at_ratact),
    ("%RATSW", QuectelBg770a::at_ratsw),
    ("%SCAN", QuectelBg770a::at_scan),
    ("%SCANCFG", QuectelBg770a::at_scancfg),
    ("%STATEV", QuectelBg770a::at_statev),
    ("%STATUS", QuectelBg770a::at_status),
    ("+CCLK", QuectelBg770a::at_cclk),
    ("+CEDRXS", QuectelBg770a::at_cedrxs),
    ("+CEINFO", QuectelBg770a::at_ceinfo),
    ("+CEREG", QuectelBg770a::at_cereg),
    ("+CFUN", QuectelBg770a::at_cfun),
    ("+CGACT", QuectelBg770a::at_cgact),
    ("+CGATT", QuectelBg770a::at_cgatt),
    ("+CGDCONT", QuectelBg770a::at_cgdcont),
    ("+CGMI", QuectelBg770a::at_cgmi),
    ("+CGMM", QuectelBg770a::at_cgmm),
    ("+CGMR", QuectelBg770a::at_cgmr),
    ("+CGPADDR", QuectelBg770a::at_cgpaddr),
    ("+CGSN", QuectelBg770a::at_cgsn),
    ("+CIMI", QuectelBg770a::at_cimi),
    ("+CMEE", QuectelBg770a::at_cmee),
    ("+CMGD", QuectelBg770a::at_cmgd),
    ("+CMGF", QuectelBg770a::at_cmgf),
    ("+CMGL", QuectelBg770a::at_cmgl),
    ("+CMGR", QuectelBg770a::at_cmgr),
    ("+CMGS", QuectelBg770a::at_cmgs),
    ("+CNMI", QuectelBg770a::at_cnmi),
    ("+COPS", QuectelBg770a::at_cops),
    ("+CPIN", QuectelBg770a::at_cpin),
    ("+CPSMS", QuectelBg770a::at_cpsms),
    ("+CREG", QuectelBg770a::at_creg),
    ("+CSCA", QuectelBg770a::at_csca),
    ("+CSCS", QuectelBg770a::at_cscs),
    ("+CSQ", QuectelBg770a::at_csq),
    ("+GMI", QuectelBg770a::at_gmi),
    ("+GMM", QuectelBg770a::at_gmm),
    ("+GMR", QuectelBg770a::at_gmr),
    ("+QCCID", QuectelBg770a::at_qccid),
    ("+QCSQ", QuectelBg770a::at_qcsq),
    ("+QENG", QuectelBg770a::at_qeng),
    ("+QFCLOSE", QuectelBg770a::at_qfclose),
    ("+QFDEL", QuectelBg770a::at_qfdel),
    ("+QFDWL", QuectelBg770a::at_qfdwl),
    ("+QFLDS", QuectelBg770a::at_qflds),
    ("+QFLST", QuectelBg770a::at_qflst),
    ("+QFOPEN", QuectelBg770a::at_qfopen),
    ("+QFOTADL", QuectelBg770a::at_qfotadl),
    ("+QFREAD", QuectelBg770a::at_qfread),
    ("+QFUPL", QuectelBg770a::at_qfupl),
    ("+QFWRITE", QuectelBg770a::at_qfwrite),
    ("+QGPS", QuectelBg770a::at_qgps),
    ("+QGPSCFG", QuectelBg770a::at_qgpscfg),
    ("+QGPSEND", QuectelBg770a::at_qgpsend),
    ("+QGPSLOC", QuectelBg770a::at_qgpsloc),
    ("+QHTTPCFG", QuectelBg770a::at_qhttpcfg),
    ("+QHTTPGET", QuectelBg770a::at_qhttpget),
    ("+QHTTPPOST", QuectelBg770a::at_qhttppost),
    ("+QHTTPREAD", QuectelBg770a::at_qhttpread),
    ("+QHTTPURL", QuectelBg770a::at_qhttpurl),
    ("+QHVN", QuectelBg770a::at_qhvn),
    ("+QIACT", QuectelBg770a::at_qiact),
    ("+QICLOSE", QuectelBg770a::at_qiclose),
    ("+QICSGP", QuectelBg770a::at_qicsgp),
    ("+QIDEACT", QuectelBg770a::at_at_qideact),
    ("+QIDNSCFG", QuectelBg770a::at_qidnscfg),
    ("+QIDNSGIP", QuectelBg770a::at_qidnsgip),
    ("+QIGETERROR", QuectelBg770a::at_qigeterror),
    ("+QIOPEN", QuectelBg770a::at_qiopen),
    ("+QIRD", QuectelBg770a::at_qird),
    ("+QISEND", QuectelBg770a::at_qisend),
    ("+QISTATE", QuectelBg770a::at_qistate),
    ("+QKTFOTA", QuectelBg770a::at_qktfota),
    ("+QLBS", QuectelBg770a::at_qlbs),
    ("+QLBSCFG", QuectelBg770a::at_qlbscfg),
    ("+QLTS", QuectelBg770a::at_qlts),
    ("+QMTCFG", QuectelBg770a::at_qmtcfg),
    ("+QMTCLOSE", QuectelBg770a::at_qmtclose),
    ("+QMTCONN", QuectelBg770a::at_qmtconn),
    ("+QMTDISC", QuectelBg770a::at_qmtdisc),
    ("+QMTOPEN", QuectelBg770a::at_qmtopen),
    ("+QMTPUB", QuectelBg770a::at_qmtpub),
    ("+QMTSUB", QuectelBg770a::at_qmtsub),
    ("+QNTP", QuectelBg770a::at_qntp),
    ("+QNWINFO", QuectelBg770a::at_qnwinfo),
    ("+QPING", QuectelBg770a::at_qping),
    ("+QPOWD", QuectelBg770a::at_at_qpowd),
    ("+QPSMCFG", QuectelBg770a::at_qpsmcfg),
    ("+QSCLK", QuectelBg770a::at_qsclk),
    ("+QSSLCFG", QuectelBg770a::at_qsslcfg),
    ("+QSSLCLOSE", QuectelBg770a::at_qsslclose),
    ("+QSSLOPEN", QuectelBg770a::at_qsslopen),
    ("+QSSLRECV", QuectelBg770a::at_qsslrecv),
    ("+QSSLSEND", QuectelBg770a::at_qsslsend),
    ("+QSSLSTATE", QuectelBg770a::at_qsslstate),
    ("+VZWAPNE", QuectelBg770a::at_vzwapne),
    ("I", QuectelBg770a::at_i),
    ("I1", QuectelBg770a::at_i1),
];

/// Look up the handler for a parsed request. Returns `None` when no entry
/// matches, which `handle_line` turns into the catch-all `ERROR` path.
pub fn handler_for(name: &str) -> Option<AtHandler> {
    AT_HANDLERS
        .binary_search_by(|(n, _)| (*n).cmp(name))
        .ok()
        .map(|i| AT_HANDLERS[i].1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_at_test_form() {
        let r = parse_at("AT+CPIN=?").unwrap();
        assert_eq!(r.name, "+CPIN");
        assert_eq!(r.form, AtForm::Test);
    }

    #[test]
    fn parse_at_read_form() {
        let r = parse_at("AT+CPIN?").unwrap();
        assert_eq!(r.name, "+CPIN");
        assert_eq!(r.form, AtForm::Read);
    }

    #[test]
    fn parse_at_write_form() {
        let r = parse_at("AT+CPIN=1234").unwrap();
        assert_eq!(r.name, "+CPIN");
        assert_eq!(r.form, AtForm::Write("1234"));
    }

    #[test]
    fn parse_at_exec_form() {
        let r = parse_at("AT+CSQ").unwrap();
        assert_eq!(r.name, "+CSQ");
        assert_eq!(r.form, AtForm::Exec);
    }

    #[test]
    fn parse_at_bare_at() {
        let r = parse_at("AT").unwrap();
        assert_eq!(r.name, "");
        assert_eq!(r.form, AtForm::Exec);
    }

    #[test]
    fn parse_at_empty_write_args() {
        let r = parse_at("AT+CGDCONT=").unwrap();
        assert_eq!(r.name, "+CGDCONT");
        assert_eq!(r.form, AtForm::Write(""));
    }

    #[test]
    fn parse_at_args_may_contain_question_and_equals() {
        // The split is at the FIRST delimiter, so later `?`/`=` stay in the arg.
        let r = parse_at("AT+QMTPUB=0,1,1,0,\"a?b=c\"").unwrap();
        assert_eq!(r.name, "+QMTPUB");
        assert_eq!(r.form, AtForm::Write("0,1,1,0,\"a?b=c\""));
    }

    #[test]
    fn parse_at_percent_and_no_plus_names() {
        let r = parse_at("AT%RATACT?").unwrap();
        assert_eq!(r.name, "%RATACT");
        assert_eq!(r.form, AtForm::Read);
        let r = parse_at("ATI1").unwrap();
        assert_eq!(r.name, "I1");
        assert_eq!(r.form, AtForm::Exec);
    }

    #[test]
    fn parse_at_lowercase_input_keeps_case() {
        // The parser is case-agnostic; callers normally hand it the uppercased
        // line, but the name/arg reflect whatever they passed in.
        let r = parse_at("at+cpin=AbC").unwrap();
        assert_eq!(r.name, "+cpin");
        assert_eq!(r.form, AtForm::Write("AbC"));
    }

    #[test]
    fn parse_at_trailing_garbage_after_question_is_not_a_request() {
        assert!(parse_at("AT+CSQ?x").is_none());
        // `AT+X=??` is a write of `??`, not a test form.
        let r = parse_at("AT+CPIN=??").unwrap();
        assert_eq!(r.form, AtForm::Write("??"));
        // Not an AT line at all.
        assert!(parse_at("BOGUS").is_none());
    }

    #[test]
    fn at_handlers_table_is_sorted_and_unique() {
        let mut prev: Option<&str> = None;
        for (name, _) in AT_HANDLERS {
            if let Some(p) = prev {
                assert!(
                    p < *name,
                    "AT_HANDLERS not sorted/unique: {p:?} then {name:?}"
                );
            }
            prev = Some(name);
        }
    }

    #[test]
    fn handler_for_finds_known_names_case_sensitively() {
        assert!(handler_for("+CPIN").is_some());
        assert!(handler_for("%RATACT").is_some());
        assert!(handler_for("I").is_some());
        // Names arrive uppercased from handle_line; lowercase is a miss.
        assert!(handler_for("+cpin").is_none());
        assert!(handler_for("+NOPE").is_none());
    }
}
