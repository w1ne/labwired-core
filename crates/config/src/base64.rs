// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RFC 4648 base64 (standard alphabet, `=` padding) for binary blobs carried
//! inline in a manifest, such as a disk image a device kit reads.
//!
//! The engine core has no filesystem (it also runs in the browser), so a file a
//! manifest names is read by [`crate::SystemManifest::from_file`] and handed to
//! the core inline as text; this is the encoding on both ends.

use anyhow::{bail, Result};

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Decode, ignoring ASCII whitespace (YAML folds long scalars).
pub fn decode(text: &str) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0u32;
    let mut padding = 0usize;
    for (i, c) in text.bytes().enumerate() {
        if c.is_ascii_whitespace() {
            continue;
        }
        if c == b'=' {
            padding += 1;
            continue;
        }
        if padding > 0 {
            bail!("base64: data after padding at offset {i}");
        }
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => bail!("base64: invalid character {:?} at offset {i}", c as char),
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    if padding > 2 || bits >= 6 {
        bail!("base64: truncated input");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 4648 section 10 test vectors.
    #[test]
    fn rfc4648_vectors_round_trip() {
        for (plain, enc) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(encode(plain.as_bytes()), enc);
            assert_eq!(decode(enc).unwrap(), plain.as_bytes());
        }
    }

    #[test]
    fn decode_skips_whitespace_and_rejects_garbage() {
        assert_eq!(decode("Zm9v\n YmFy").unwrap(), b"foobar");
        assert!(decode("Zm9v*").is_err());
        assert!(decode("Zg==Zg").is_err());
    }

    #[test]
    fn binary_round_trip() {
        let data: Vec<u8> = (0..=255u8).cycle().take(1027).collect();
        assert_eq!(decode(&encode(&data)).unwrap(), data);
    }
}
