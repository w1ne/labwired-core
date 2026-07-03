// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Encode a batch of [`EgressItem`]s into a single on-wire payload.

use crate::network::egress::{EgressItem, EncodingKind};
use crate::network::CanFrame;

/// Encode `items` into one payload according to `kind`. Empty in → empty out.
pub fn encode(kind: EncodingKind, items: &[EgressItem]) -> Vec<u8> {
    match kind {
        EncodingKind::Raw => {
            let mut out = Vec::new();
            for item in items {
                match item {
                    EgressItem::Byte(b) => out.push(*b),
                    EgressItem::Frame(f) => out.extend_from_slice(&f.data),
                }
            }
            out
        }
        EncodingKind::NdjsonTrace => {
            let mut out = String::new();
            for item in items {
                match item {
                    EgressItem::Byte(b) => {
                        out.push_str(&format!("{{\"kind\":\"byte\",\"byte\":{b}}}\n"));
                    }
                    EgressItem::Frame(f) => {
                        out.push_str(&format!("{{\"kind\":\"frame\",{}}}\n", frame_fields(f)));
                    }
                }
            }
            out.into_bytes()
        }
        EncodingKind::FramesJson => {
            let objs: Vec<String> = items
                .iter()
                .filter_map(|item| match item {
                    EgressItem::Frame(f) => Some(format!("{{{}}}", frame_fields(f))),
                    EgressItem::Byte(_) => None,
                })
                .collect();
            if objs.is_empty() {
                Vec::new()
            } else {
                format!("[{}]", objs.join(",")).into_bytes()
            }
        }
    }
}

/// Shared `id,data,extended,fd` field list for a CAN frame (no braces).
fn frame_fields(f: &CanFrame) -> String {
    let data: Vec<String> = f.data.iter().map(|b| b.to_string()).collect();
    format!(
        "\"id\":{},\"data\":[{}],\"extended\":{},\"fd\":{}",
        f.id,
        data.join(","),
        f.extended,
        f.fd
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::egress::{EgressItem, EncodingKind};
    use crate::network::CanFrame;

    #[test]
    fn raw_concatenates_bytes() {
        let items = vec![EgressItem::Byte(b'h'), EgressItem::Byte(b'i')];
        assert_eq!(encode(EncodingKind::Raw, &items), b"hi".to_vec());
    }

    #[test]
    fn ndjson_emits_one_line_per_item() {
        let items = vec![EgressItem::Byte(0x41)];
        let out = String::from_utf8(encode(EncodingKind::NdjsonTrace, &items)).unwrap();
        assert_eq!(out, "{\"kind\":\"byte\",\"byte\":65}\n");
    }

    #[test]
    fn frames_json_is_array_and_skips_bytes() {
        let items = vec![
            EgressItem::Byte(0x00),
            EgressItem::Frame(CanFrame::classic(0x123, vec![1, 2])),
        ];
        let out = String::from_utf8(encode(EncodingKind::FramesJson, &items)).unwrap();
        assert_eq!(
            out,
            "[{\"id\":291,\"data\":[1,2],\"extended\":false,\"fd\":false}]"
        );
    }

    #[test]
    fn empty_input_is_empty() {
        assert!(encode(EncodingKind::Raw, &[]).is_empty());
    }
}
