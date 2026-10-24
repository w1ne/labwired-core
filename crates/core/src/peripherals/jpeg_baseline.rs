// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! A small **baseline JPEG encoder** (ITU-T T.81, sequential DCT, Huffman), for
//! camera models that have to put a real JPEG bitstream on their pixel bus.
//!
//! A sensor in JPEG mode is judged by what the firmware does with its output:
//! `esp32-camera` looks for the `FF D8 FF` start marker in the first DMA block
//! and for the `FF D9` end marker to size the frame, and the application then
//! hands the buffer to a JPEG decoder or ships it over HTTP. A placeholder
//! byte pattern with the right markers would satisfy the first and fail the
//! second, so this module emits a stream any baseline decoder accepts:
//!
//! * JFIF APP0, two quantisation tables (luma / chroma), SOF0, the four
//!   standard Huffman tables of Annex K.3, one interleaved scan, EOI;
//! * YCbCr 4:2:2 (two luma blocks per chroma pair, 16×8 MCUs), which is the
//!   sampling OmniVision sensors use for their JPEG output;
//! * quantisation from the Annex K tables scaled by the familiar 1–100
//!   quality (the libjpeg mapping), so a lower quality really is smaller;
//! * edge MCUs padded by replicating the last row/column, as encoders do.
//!
//! It is written for clarity, not speed (a separable floating-point DCT), which
//! is plenty for the frame sizes a twin streams.

/// Annex K.1 luminance quantisation table, natural (row-major) order.
const LUMA_Q: [u8; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61, //
    12, 12, 14, 19, 26, 58, 60, 55, //
    14, 13, 16, 24, 40, 57, 69, 56, //
    14, 17, 22, 29, 51, 87, 80, 62, //
    18, 22, 37, 56, 68, 109, 103, 77, //
    24, 35, 55, 64, 81, 104, 113, 92, //
    49, 64, 78, 87, 103, 121, 120, 101, //
    72, 92, 95, 98, 112, 100, 103, 99,
];

/// Annex K.1 chrominance quantisation table, natural order.
const CHROMA_Q: [u8; 64] = [
    17, 18, 24, 47, 99, 99, 99, 99, //
    18, 21, 26, 66, 99, 99, 99, 99, //
    24, 26, 56, 99, 99, 99, 99, 99, //
    47, 66, 99, 99, 99, 99, 99, 99, //
    99, 99, 99, 99, 99, 99, 99, 99, //
    99, 99, 99, 99, 99, 99, 99, 99, //
    99, 99, 99, 99, 99, 99, 99, 99, //
    99, 99, 99, 99, 99, 99, 99, 99,
];

/// Zig-zag index → natural index.
const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

// Annex K.3 Huffman tables: BITS (codes per length 1..=16) then HUFFVAL.
const DC_LUMA_BITS: [u8; 16] = [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];
const DC_LUMA_VALS: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
const DC_CHROMA_BITS: [u8; 16] = [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0];
const DC_CHROMA_VALS: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
const AC_LUMA_BITS: [u8; 16] = [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7d];
const AC_LUMA_VALS: [u8; 162] = [
    0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07,
    0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xa1, 0x08, 0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52, 0xd1, 0xf0,
    0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0a, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x25, 0x26, 0x27, 0x28,
    0x29, 0x2a, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49,
    0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69,
    0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89,
    0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7,
    0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5,
    0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe1, 0xe2,
    0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8,
    0xf9, 0xfa,
];
const AC_CHROMA_BITS: [u8; 16] = [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77];
const AC_CHROMA_VALS: [u8; 162] = [
    0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71,
    0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xa1, 0xb1, 0xc1, 0x09, 0x23, 0x33, 0x52, 0xf0,
    0x15, 0x62, 0x72, 0xd1, 0x0a, 0x16, 0x24, 0x34, 0xe1, 0x25, 0xf1, 0x17, 0x18, 0x19, 0x1a, 0x26,
    0x27, 0x28, 0x29, 0x2a, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48,
    0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68,
    0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87,
    0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5,
    0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3,
    0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda,
    0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8,
    0xf9, 0xfa,
];

/// A Huffman code table indexed by symbol: `(code, length)`.
struct HuffTable {
    codes: [(u16, u8); 256],
}

impl HuffTable {
    /// Annex C: generate the code for each symbol from BITS / HUFFVAL.
    fn new(bits: &[u8; 16], vals: &[u8]) -> Self {
        let mut codes = [(0u16, 0u8); 256];
        let mut code: u16 = 0;
        let mut k = 0;
        for (len_minus_one, &count) in bits.iter().enumerate() {
            for _ in 0..count {
                codes[vals[k] as usize] = (code, len_minus_one as u8 + 1);
                code += 1;
                k += 1;
            }
            code <<= 1;
        }
        Self { codes }
    }
}

/// Entropy-coded segment writer with `0xFF` byte stuffing.
struct BitWriter {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

impl BitWriter {
    fn put(&mut self, code: u16, len: u8) {
        self.acc = (self.acc << len) | u32::from(code);
        self.n += u32::from(len);
        while self.n >= 8 {
            let byte = (self.acc >> (self.n - 8)) as u8;
            self.out.push(byte);
            if byte == 0xFF {
                self.out.push(0x00);
            }
            self.n -= 8;
        }
        self.acc &= (1 << self.n) - 1;
    }

    /// Pad the final byte with 1-bits (T.81 F.1.2.3).
    fn flush(&mut self) {
        if self.n > 0 {
            let pad = 8 - self.n;
            self.put((1 << pad) - 1, pad as u8);
        }
    }
}

/// Quality 1..=100 → table scaled the way libjpeg does it.
fn scale_table(base: &[u8; 64], quality: u8) -> [u8; 64] {
    let q = u32::from(quality.clamp(1, 100));
    let s = if q < 50 { 5000 / q } else { 200 - 2 * q };
    let mut t = [0u8; 64];
    for (o, &b) in t.iter_mut().zip(base) {
        *o = ((u32::from(b) * s + 50) / 100).clamp(1, 255) as u8;
    }
    t
}

/// Separable 8×8 forward DCT (T.81 A.3.3), input level-shifted.
fn fdct(block: &[f32; 64]) -> [f32; 64] {
    let mut cos = [[0f32; 8]; 8];
    for (u, row) in cos.iter_mut().enumerate() {
        for (x, c) in row.iter_mut().enumerate() {
            *c = (((2 * x + 1) as f32 * u as f32 * std::f32::consts::PI) / 16.0).cos();
        }
    }
    let alpha = |u: usize| {
        if u == 0 {
            std::f32::consts::FRAC_1_SQRT_2
        } else {
            1.0
        }
    };
    let mut tmp = [0f32; 64];
    for y in 0..8 {
        for u in 0..8 {
            let s: f32 = (0..8).map(|x| block[y * 8 + x] * cos[u][x]).sum();
            tmp[y * 8 + u] = 0.5 * alpha(u) * s;
        }
    }
    let mut out = [0f32; 64];
    for u in 0..8 {
        for v in 0..8 {
            let s: f32 = (0..8).map(|y| tmp[y * 8 + u] * cos[v][y]).sum();
            out[v * 8 + u] = 0.5 * alpha(v) * s;
        }
    }
    out
}

/// Magnitude category and the extra bits for a coefficient (T.81 F.1.2.1).
fn category(v: i32) -> (u8, u16) {
    let a = v.unsigned_abs();
    let cat = (32 - a.leading_zeros()) as u8;
    let bits = if v < 0 {
        (v - 1) as u32 & ((1 << cat) - 1)
    } else {
        v as u32
    };
    (cat, bits as u16)
}

struct Component<'a> {
    q: [u8; 64],
    dc: &'a HuffTable,
    ac: &'a HuffTable,
    pred: i32,
}

impl Component<'_> {
    fn encode_block(&mut self, w: &mut BitWriter, samples: &[f32; 64]) {
        let coef = fdct(samples);
        let mut zz = [0i32; 64];
        for (k, &n) in ZIGZAG.iter().enumerate() {
            zz[k] = (coef[n] / f32::from(self.q[n])).round() as i32;
        }
        let diff = zz[0] - self.pred;
        self.pred = zz[0];
        let (cat, bits) = category(diff);
        let (code, len) = self.dc.codes[cat as usize];
        w.put(code, len);
        if cat > 0 {
            w.put(bits, cat);
        }
        let mut run = 0;
        for &c in &zz[1..] {
            if c == 0 {
                run += 1;
                continue;
            }
            while run > 15 {
                let (code, len) = self.ac.codes[0xF0];
                w.put(code, len);
                run -= 16;
            }
            let (cat, bits) = category(c);
            let (code, len) = self.ac.codes[(run << 4 | cat) as usize];
            w.put(code, len);
            w.put(bits, cat);
            run = 0;
        }
        if run > 0 {
            let (code, len) = self.ac.codes[0x00];
            w.put(code, len);
        }
    }
}

fn segment(out: &mut Vec<u8>, marker: u8, body: &[u8]) {
    out.extend_from_slice(&[0xFF, marker]);
    out.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(body);
}

fn dht(class_id: u8, bits: &[u8; 16], vals: &[u8]) -> Vec<u8> {
    let mut b = vec![class_id];
    b.extend_from_slice(bits);
    b.extend_from_slice(vals);
    b
}

/// BT.601 full-range RGB → YCbCr.
fn ycbcr((r, g, b): (u8, u8, u8)) -> (f32, f32, f32) {
    let (r, g, b) = (f32::from(r), f32::from(g), f32::from(b));
    (
        0.299 * r + 0.587 * g + 0.114 * b,
        -0.168_736 * r - 0.331_264 * g + 0.5 * b + 128.0,
        0.5 * r - 0.418_688 * g - 0.081_312 * b + 128.0,
    )
}

/// Encode a `width` × `height` image whose pixels `pixel(x, y)` returns as
/// RGB, at `quality` 1..=100. Returns a complete JFIF file, `FF D8` … `FF D9`.
///
/// Width and height must be non-zero and fit the 16-bit SOF0 fields.
pub fn encode_rgb(
    width: u32,
    height: u32,
    quality: u8,
    mut pixel: impl FnMut(u32, u32) -> (u8, u8, u8),
) -> Vec<u8> {
    assert!(width > 0 && height > 0 && width <= 0xFFFF && height <= 0xFFFF);
    let lq = scale_table(&LUMA_Q, quality);
    let cq = scale_table(&CHROMA_Q, quality);
    let mut out = Vec::with_capacity((width * height / 4) as usize + 1024);
    out.extend_from_slice(&[0xFF, 0xD8]);
    segment(
        &mut out,
        0xE0,
        &[b'J', b'F', b'I', b'F', 0, 1, 1, 0, 0, 1, 0, 1, 0, 0],
    );
    for (id, table) in [(0u8, &lq), (1u8, &cq)] {
        let mut body = vec![id];
        body.extend(ZIGZAG.iter().map(|&n| table[n]));
        segment(&mut out, 0xDB, &body);
    }
    let (w16, h16) = (width as u16, height as u16);
    let mut sof = vec![8];
    sof.extend_from_slice(&h16.to_be_bytes());
    sof.extend_from_slice(&w16.to_be_bytes());
    // Three components: Y sampled 2×1 (4:2:2), Cb / Cr 1×1.
    sof.extend_from_slice(&[3, 1, 0x21, 0, 2, 0x11, 1, 3, 0x11, 1]);
    segment(&mut out, 0xC0, &sof);
    segment(&mut out, 0xC4, &dht(0x00, &DC_LUMA_BITS, &DC_LUMA_VALS));
    segment(&mut out, 0xC4, &dht(0x10, &AC_LUMA_BITS, &AC_LUMA_VALS));
    segment(&mut out, 0xC4, &dht(0x01, &DC_CHROMA_BITS, &DC_CHROMA_VALS));
    segment(&mut out, 0xC4, &dht(0x11, &AC_CHROMA_BITS, &AC_CHROMA_VALS));
    segment(&mut out, 0xDA, &[3, 1, 0x00, 2, 0x11, 3, 0x11, 0, 63, 0]);

    let dc_l = HuffTable::new(&DC_LUMA_BITS, &DC_LUMA_VALS);
    let ac_l = HuffTable::new(&AC_LUMA_BITS, &AC_LUMA_VALS);
    let dc_c = HuffTable::new(&DC_CHROMA_BITS, &DC_CHROMA_VALS);
    let ac_c = HuffTable::new(&AC_CHROMA_BITS, &AC_CHROMA_VALS);
    let mut y = Component {
        q: lq,
        dc: &dc_l,
        ac: &ac_l,
        pred: 0,
    };
    let mut cb = Component {
        q: cq,
        dc: &dc_c,
        ac: &ac_c,
        pred: 0,
    };
    let mut cr = Component {
        q: cq,
        dc: &dc_c,
        ac: &ac_c,
        pred: 0,
    };
    let mut bw = BitWriter { out, acc: 0, n: 0 };

    let mut mcu = vec![(0f32, 0f32, 0f32); 16 * 8];
    for my in (0..height).step_by(8) {
        for mx in (0..width).step_by(16) {
            for dy in 0..8 {
                for dx in 0..16 {
                    let px = (mx + dx).min(width - 1);
                    let py = (my + dy).min(height - 1);
                    mcu[(dy * 16 + dx) as usize] = ycbcr(pixel(px, py));
                }
            }
            for half in 0..2 {
                let mut blk = [0f32; 64];
                for (i, s) in blk.iter_mut().enumerate() {
                    *s = mcu[(i / 8) * 16 + half * 8 + i % 8].0 - 128.0;
                }
                y.encode_block(&mut bw, &blk);
            }
            let (mut bcb, mut bcr) = ([0f32; 64], [0f32; 64]);
            for i in 0..64 {
                let (a, b) = (
                    mcu[(i / 8) * 16 + (i % 8) * 2],
                    mcu[(i / 8) * 16 + (i % 8) * 2 + 1],
                );
                bcb[i] = (a.1 + b.1) / 2.0 - 128.0;
                bcr[i] = (a.2 + b.2) / 2.0 - 128.0;
            }
            cb.encode_block(&mut bw, &bcb);
            cr.encode_block(&mut bw, &bcr);
        }
    }
    bw.flush();
    let mut out = bw.out;
    out.extend_from_slice(&[0xFF, 0xD9]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(data: &[u8]) -> (jpeg_decoder::ImageInfo, Vec<u8>) {
        let mut d = jpeg_decoder::Decoder::new(data);
        let pixels = d.decode().expect("a baseline decoder accepts the stream");
        (d.info().expect("header parsed"), pixels)
    }

    #[test]
    fn output_is_a_complete_jfif_file_a_decoder_accepts() {
        let jpg = encode_rgb(
            37,
            21,
            80,
            |x, _| if x < 18 { (255, 0, 0) } else { (0, 0, 255) },
        );
        assert_eq!(&jpg[..3], &[0xFF, 0xD8, 0xFF], "SOI then a marker");
        assert_eq!(&jpg[jpg.len() - 2..], &[0xFF, 0xD9], "ends with EOI");
        let (info, px) = decode(&jpg);
        assert_eq!((info.width, info.height), (37, 21));
        assert_eq!(info.pixel_format, jpeg_decoder::PixelFormat::RGB24);
        // Left half red, right half blue, within JPEG error.
        let at = |x: usize, y: usize| &px[(y * 37 + x) * 3..(y * 37 + x) * 3 + 3];
        let (l, r) = (at(4, 10), at(32, 10));
        assert!(l[0] > 200 && l[2] < 60, "left is red: {l:?}");
        assert!(r[2] > 200 && r[0] < 60, "right is blue: {r:?}");
    }

    #[test]
    fn lower_quality_is_smaller() {
        let img = |x: u32, y: u32| ((x * 7) as u8, (y * 5) as u8, ((x ^ y) * 3) as u8);
        let hi = encode_rgb(160, 120, 90, img);
        let lo = encode_rgb(160, 120, 20, img);
        assert!(lo.len() < hi.len(), "q20 {} < q90 {}", lo.len(), hi.len());
        decode(&hi);
        decode(&lo);
    }

    #[test]
    fn entropy_data_never_contains_a_bare_marker() {
        // A busy image exercises long codes; every 0xFF in the scan must be
        // stuffed, so the only FF D9 in the file is the real EOI.
        let jpg = encode_rgb(64, 64, 95, |x, y| {
            (((x * 37) ^ (y * 91)) as u8, (x * y) as u8, 255)
        });
        let eois = jpg.windows(2).filter(|w| w == &[0xFF, 0xD9]).count();
        assert_eq!(eois, 1);
        decode(&jpg);
    }
}
