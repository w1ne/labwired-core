//! candump `.log` parser for the `can-player` external device.
//!
//! Exactly one input format, forever (see 2026-07-02 replay-showcase spec):
//! the SocketCAN candump log line `(<ts>) <iface> <ID>#<DATA>`. Extended
//! (29-bit) identifiers are written as 8 hex digits, standard as 3. CAN-FD
//! lines (`##`) are rejected — classical CAN only.

use crate::network::CanFrame;

pub fn parse_candump(text: &str) -> Result<Vec<(f64, CanFrame)>, String> {
    let mut out = Vec::new();
    for (lineno, raw) in text.lines().enumerate() {
        let n = lineno + 1;
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let (Some(ts), Some(_iface), Some(frame)) = (parts.next(), parts.next(), parts.next())
        else {
            return Err(format!("candump line {n}: expected '(<ts>) <iface> <ID>#<DATA>'"));
        };
        let ts: f64 = ts
            .strip_prefix('(')
            .and_then(|t| t.strip_suffix(')'))
            .ok_or_else(|| format!("candump line {n}: timestamp must be '(seconds)'"))?
            .parse()
            .map_err(|e| format!("candump line {n}: bad timestamp: {e}"))?;
        if frame.contains("##") {
            return Err(format!(
                "candump line {n}: CAN-FD frames ('##') are not supported; can-player is classical CAN only"
            ));
        }
        let (id_str, data_str) = frame
            .split_once('#')
            .ok_or_else(|| format!("candump line {n}: expected '<ID>#<DATA>'"))?;
        let id = u32::from_str_radix(id_str, 16)
            .map_err(|e| format!("candump line {n}: bad CAN id '{id_str}': {e}"))?;
        // can-utils convention: 8 hex digits = 29-bit extended, 3 = standard.
        let extended = id_str.len() > 3;
        if data_str.len() % 2 != 0 {
            return Err(format!("candump line {n}: odd-length hex payload"));
        }
        if !data_str.is_ascii() {
            return Err(format!("candump line {n}: non-ASCII characters in payload"));
        }
        let data = (0..data_str.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&data_str[i..i + 2], 16))
            .collect::<Result<Vec<u8>, _>>()
            .map_err(|e| format!("candump line {n}: bad hex payload: {e}"))?;
        out.push((
            ts,
            CanFrame {
                id,
                data,
                extended,
                fd: false,
                bitrate_switch: false,
                remote: false,
            },
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_extended_and_standard_frames() {
        let log = "(1578925458.824500) can0 0CF00300#DD0000FFFFFF5CFF\n\
                   (1578925458.825100) can0 123#DEADBEEF\n";
        let frames = parse_candump(log).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].0, 1578925458.8245);
        assert_eq!(frames[0].1.id, 0x0CF00300);
        assert!(frames[0].1.extended);
        assert_eq!(frames[0].1.data, vec![0xDD, 0, 0, 0xFF, 0xFF, 0xFF, 0x5C, 0xFF]);
        assert_eq!(frames[1].1.id, 0x123);
        assert!(!frames[1].1.extended);
        assert_eq!(frames[1].1.data.len(), 4);
    }

    #[test]
    fn skips_blank_lines() {
        let log = "\n(1.0) can0 123#11\n\n";
        assert_eq!(parse_candump(log).unwrap().len(), 1);
    }

    #[test]
    fn zero_length_payload_ok() {
        let frames = parse_candump("(1.0) can0 123#\n").unwrap();
        assert!(frames[0].1.data.is_empty());
    }

    #[test]
    fn rejects_fd_frames_with_line_number() {
        let err = parse_candump("(1.0) can0 123##188\n").unwrap_err();
        assert!(err.contains("line 1"), "got: {err}");
        assert!(err.to_lowercase().contains("fd"));
    }

    #[test]
    fn rejects_malformed_line_with_line_number() {
        let err = parse_candump("(1.0) can0\nnot a line\n").unwrap_err();
        assert!(err.contains("line 1"), "got: {err}");
    }

    #[test]
    fn rejects_odd_hex_payload() {
        assert!(parse_candump("(1.0) can0 123#ABC\n").is_err());
    }

    #[test]
    fn rejects_non_ascii_payload_gracefully() {
        // 'A' (1 byte) + '€' (3 bytes) = 4 bytes, passes the even-length check,
        // but must produce Err, not a char-boundary panic.
        let err = parse_candump("(1.0) can0 123#A€\n").unwrap_err();
        assert!(err.contains("line 1"), "got: {err}");
    }
}
