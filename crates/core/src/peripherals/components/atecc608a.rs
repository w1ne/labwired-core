// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! ATECC608A-style secure element (I²C, default address 0x60).
//!
//! The "external TPM / HSM" for security labs: key slots that never leave the
//! chip, ECDSA P-256 sign/verify over a nonce-loaded digest, a deterministic
//! RNG, and a read-only device identity. Realistic enough to drive the full
//! signed-OTA flow: firmware hashes the update, the SE verifies the OEM's
//! signature against a public key stored in its data zone.
//!
//! ## Fidelity contract
//!
//! - Packet framing follows the real ATECC608A single-wire/I²C format:
//!   `[count, opcode, p1, p2le, …data, crc16le]` on write, `[count, …resp,
//!   crc16le]` on read, CRC16-CCITT (poly 0x8005, reflected, init 0) covering
//!   `count` through the last data byte. Wake/idle sleep cycles and the
//!   execution-time polling of real silicon are NOT modelled: every command
//!   completes when its packet lands.
//! - Implemented opcodes (real numbers): INFO 0x30, RANDOM 0x1B, READ 0x02,
//!   NONCE 0x16 (passthrough digest load), VERIFY 0x45 (external key),
//!   SIGN 0x41 (device key over loaded digest).
//! - Crypto is REAL, not mocked: ECDSA P-256 verify/sign via the `p256`
//!   crate (`verify_prehash` / RFC 6979 `sign_prehash`), RNG is a SHA-256
//!   counter chain from a fixed seed — deterministic across runs, like the
//!   nRF52 RNG model, so tests can assert golden values.
//!
//! ## Keys
//!
//! - Data slot 0: OEM **public** key — the update-signing authority. Only the
//!   public half exists on the device. Override via system.yaml
//!   `config.oem_pubkey_hex` (128 hex chars = 64-byte X‖Y). Default is a
//!   well-known demo pubkey so pre-signed playground packages keep working;
//!   the matching private key is never committed — generate packages with
//!   `make_packages.py --ephemeral` or `--key`.
//! - Device key: a fixed P-256 keypair the SE signs attestation challenges
//!   with (its private half never leaves the model — there is no command to
//!   read it).

use crate::peripherals::i2c::I2cDevice;
use p256::ecdsa::signature::hazmat::{PrehashSigner, PrehashVerifier};
use p256::ecdsa::{Signature, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};

const ADDR_DEFAULT: u8 = 0x60;

// Opcodes (real ATECC608A values).
const OP_NONCE: u8 = 0x16;
const OP_RANDOM: u8 = 0x1B;
const OP_READ: u8 = 0x02;
const OP_INFO: u8 = 0x30;
const OP_SIGN: u8 = 0x41;
const OP_VERIFY: u8 = 0x45;

const STATUS_OK: u8 = 0x00;
const STATUS_VERIFY_FAILED: u8 = 0x01;
const STATUS_BAD_OPCODE: u8 = 0x0F;
const STATUS_BAD_CRC: u8 = 0xFF;

/// Default OEM update-signing public key (uncompressed P-256, 64 bytes X‖Y).
/// Used when system.yaml does not set `oem_pubkey_hex`. The matching private
/// key is not in this repository — regenerate signed packages with
/// `make_packages.py --ephemeral` (CI) or `--key` (local).
const DEFAULT_OEM_PUBKEY: [u8; 64] = [
    0x11, 0xf7, 0x19, 0x76, 0xee, 0xfb, 0xfc, 0xb5, 0xfa, 0xc9, 0xb1, 0x6c, 0xfb, 0x78, 0x43, 0xbf,
    0x61, 0x4f, 0xc1, 0x59, 0x46, 0xa8, 0xb2, 0x94, 0x94, 0xf2, 0x8c, 0xcd, 0x94, 0xd3, 0x22, 0x9b,
    0xfe, 0x07, 0xb6, 0xaf, 0x2b, 0x38, 0x4a, 0xd6, 0x14, 0x4f, 0xfc, 0x1a, 0xdf, 0x92, 0x1e, 0x0a,
    0x28, 0x38, 0xda, 0xbc, 0x41, 0xc5, 0xad, 0x85, 0x4d, 0x07, 0x0a, 0x86, 0x97, 0x33, 0xe4, 0xc9,
];

/// Device attestation key: the raw 32-byte P-256 private scalar. Fixed so
/// attestation signatures are reproducible across runs — the point of the
/// demo. The private half never leaves the model (no command reads it).
const DEVICE_KEY_SCALAR: [u8; 32] = [
    0x8a, 0x1b, 0xa4, 0x6b, 0x51, 0xaa, 0x40, 0x00, 0x13, 0x15, 0x9b, 0xf0, 0x82, 0x2a, 0xc9, 0x6b,
    0xf8, 0xfc, 0xad, 0xeb, 0x66, 0x3f, 0x78, 0x1b, 0xfb, 0x84, 0xd9, 0xf6, 0x3b, 0x73, 0x15, 0x8f,
];

const RNG_SEED: &[u8] = b"labwired-atecc608a-rng-v1";

/// ATECC CRC16: poly 0x8005, reflected, init 0, over count..last-data.
fn crc16(packet: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &byte in packet {
        crc ^= byte as u16;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xA001 } else { crc >> 1 };
        }
    }
    crc
}

pub struct Atecc608a {
    address: u8,
    /// OEM update-verify public key in data slot 0 (X‖Y).
    oem_pubkey: [u8; 64],
    /// Incoming packet accumulator (host → SE).
    rx: Vec<u8>,
    /// Executed response, drained by I²C reads (SE → host).
    resp: Vec<u8>,
    /// The 32-byte digest loaded by NONCE (passthrough mode).
    tempkey: [u8; 32],
    tempkey_valid: bool,
    /// Deterministic RNG counter (SHA-256 chain from RNG_SEED).
    rng_counter: u64,
    component_id: Option<String>,
}

impl Atecc608a {
    pub fn new(address: u8) -> Self {
        Self::with_oem_pubkey(address, DEFAULT_OEM_PUBKEY)
    }

    pub fn with_oem_pubkey(address: u8, oem_pubkey: [u8; 64]) -> Self {
        Self {
            address,
            oem_pubkey,
            rx: Vec::new(),
            resp: Vec::new(),
            tempkey: [0; 32],
            tempkey_valid: false,
            rng_counter: 0,
            component_id: None,
        }
    }

    fn device_signing_key(&self) -> SigningKey {
        SigningKey::from_slice(&DEVICE_KEY_SCALAR).expect("embedded scalar is valid")
    }

    fn device_pubkey(&self) -> [u8; 64] {
        let sk = self.device_signing_key();
        let ep = sk.verifying_key().to_encoded_point(false);
        let mut out = [0u8; 64];
        out.copy_from_slice(&ep.as_bytes()[1..65]);
        out
    }

    /// Execute one complete packet: [count, opcode, p1, p2le, …data, crc16].
    fn execute(&mut self, pkt: &[u8]) {
        let (body, crc_bytes) = pkt.split_at(pkt.len() - 2);
        let want_crc = u16::from_le_bytes([crc_bytes[0], crc_bytes[1]]);
        if crc16(body) != want_crc {
            self.respond(&[STATUS_BAD_CRC]);
            return;
        }
        let opcode = body[1];
        let p1 = body[2];
        let data = &body[5..];
        match opcode {
            // Revision / identity word; mirrors the real 0x00 0x00 0x60 0x03.
            OP_INFO => self.respond(&[0x00, 0x00, 0x60, 0x03]),
            // 32 deterministic random bytes (SHA-256 counter chain).
            OP_RANDOM => {
                let mut hasher = Sha256::new();
                hasher.update(RNG_SEED);
                hasher.update(self.rng_counter.to_le_bytes());
                self.rng_counter += 1;
                let out = hasher.finalize();
                self.respond(&out);
            }
            // Data-zone read: p2 = slot*2 + half (0/1: OEM pubkey, 2/3: device
            // pubkey), 32 bytes per read.
            OP_READ => {
                let slot_half = u16::from_le_bytes([body[3], body[4]]) as usize;
                let block: Option<[u8; 32]> = match slot_half {
                    0 | 1 => Some({
                        let mut b = [0u8; 32];
                        b.copy_from_slice(&self.oem_pubkey[slot_half * 32..slot_half * 32 + 32]);
                        b
                    }),
                    2 | 3 => {
                        let pk = self.device_pubkey();
                        let half = slot_half - 2;
                        let mut b = [0u8; 32];
                        b.copy_from_slice(&pk[half * 32..half * 32 + 32]);
                        Some(b)
                    }
                    _ => None,
                };
                match block {
                    Some(b) => self.respond(&b),
                    None => self.respond(&[STATUS_BAD_OPCODE]),
                }
            }
            // NONCE passthrough (p1 = 0x03): load a 32-byte digest into tempkey.
            OP_NONCE if p1 == 0x03 && data.len() == 32 => {
                self.tempkey.copy_from_slice(data);
                self.tempkey_valid = true;
                self.respond(&[STATUS_OK]);
            }
            // VERIFY external (p1 = 0x02): data = sig[64] ‖ pubkey[64],
            // verified against the tempkey digest.
            OP_VERIFY if p1 == 0x02 && data.len() == 128 => {
                let ok = self.tempkey_valid && {
                    let mut vk_bytes = [0u8; 65];
                    vk_bytes[0] = 0x04;
                    vk_bytes[1..].copy_from_slice(&data[64..]);
                    match (
                        VerifyingKey::from_sec1_bytes(&vk_bytes),
                        Signature::from_slice(&data[..64]),
                    ) {
                        (Ok(vk), Ok(sig)) => vk.verify_prehash(&self.tempkey, &sig).is_ok(),
                        _ => false,
                    }
                };
                self.respond(&[if ok { STATUS_OK } else { STATUS_VERIFY_FAILED }]);
                self.tempkey_valid = false; // one-shot, like real tempkey use
            }
            // SIGN with the device key over the tempkey digest (RFC 6979 —
            // deterministic, so attestation is reproducible in tests).
            OP_SIGN => {
                if !self.tempkey_valid {
                    self.respond(&[STATUS_VERIFY_FAILED]);
                    return;
                }
                let sk = self.device_signing_key();
                let sig: Signature = sk
                    .sign_prehash(&self.tempkey)
                    .expect("device key signs");
                self.respond(&sig.to_bytes());
                self.tempkey_valid = false;
            }
            _ => self.respond(&[STATUS_BAD_OPCODE]),
        }
    }

    /// Frame a response: [count, data..., crc16le].
    fn respond(&mut self, data: &[u8]) {
        let count = (data.len() + 3) as u8; // count includes itself + crc
        let mut out = Vec::with_capacity(count as usize);
        out.push(count);
        out.extend_from_slice(data);
        let crc = crc16(&out);
        out.extend_from_slice(&crc.to_le_bytes());
        self.resp = out;
    }
}

impl I2cDevice for Atecc608a {
    fn address(&self) -> u8 {
        self.address
    }

    fn start(&mut self) {
        // A new transaction begins: drop any partial packet. The queued
        // response survives a repeated-START read phase until drained.
        self.rx.clear();
    }

    fn write(&mut self, data: u8) {
        self.rx.push(data);
        // First byte is the packet length including itself and the CRC.
        if self.rx.len() > 1 && self.rx.len() == self.rx[0] as usize {
            let pkt = core::mem::take(&mut self.rx);
            self.execute(&pkt);
        }
    }

    fn read(&mut self) -> u8 {
        if self.resp.is_empty() {
            0xFF
        } else {
            self.resp.remove(0)
        }
    }

    fn stop(&mut self) {}
}

// ─── PeripheralKit registration ──────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct Atecc608aKit;
pub static ATECC608A_KIT: Atecc608aKit = Atecc608aKit;

static ATECC608A_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "atecc608a",
    label: "ATECC608A Secure Element",
    summary: "I²C secure element: P-256 sign/verify, RNG, key slots (HSM/TPM role).",
    detail: "ATECC608A-style command set (INFO/RANDOM/READ/NONCE/VERIFY/SIGN) with real \
             p256 ECDSA. Data slot 0 holds the OEM update-verify public key; a fixed device \
             key signs attestation challenges. Deterministic RNG (SHA-256 chain) for \
             reproducible tests.",
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: &[
        ConfigKey {
            name: "i2c_address",
            ty: ConfigType::Int,
            doc: "7-bit slave address. Defaults to 0x60.",
        },
        ConfigKey {
            name: "oem_pubkey_hex",
            ty: ConfigType::Str,
            doc: "Optional 128-char hex OEM update-verify public key (64-byte                   uncompressed P-256 X‖Y) for data slot 0. When omitted, the                   well-known demo pubkey is used.",
        },
    ],
    labs: &[],
};

impl PeripheralKit for Atecc608aKit {
    fn metadata(&self) -> &'static KitMetadata {
        &ATECC608A_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let address = ctx.i2c_address_or(ADDR_DEFAULT)?;
        let oem_pubkey = match ctx.config_str("oem_pubkey_hex") {
            Some(hex) => parse_oem_pubkey_hex(hex)?,
            None => DEFAULT_OEM_PUBKEY,
        };
        let mut dev = Atecc608a::with_oem_pubkey(address, oem_pubkey);
        dev.component_id = Some(ctx.device_id().to_string());
        ctx.attach_i2c_device(Box::new(dev))?;
        Ok(())
    }
}

/// Parse 128 hex chars (optional 0x / whitespace) into a 64-byte P-256 pubkey.
fn parse_oem_pubkey_hex(s: &str) -> anyhow::Result<[u8; 64]> {
    let cleaned: String = s
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect();
    if cleaned.len() != 128 {
        anyhow::bail!(
            "oem_pubkey_hex must be 128 hex chars (64 bytes), got {} hex digits",
            cleaned.len()
        );
    }
    let mut out = [0u8; 64];
    for i in 0..64 {
        out[i] = u8::from_str_radix(&cleaned[i * 2..i * 2 + 2], 16)
            .map_err(|e| anyhow::anyhow!("oem_pubkey_hex: {e}"))?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn send(dev: &mut Atecc608a, body_without_crc: &[u8]) -> Vec<u8> {
        let mut pkt = body_without_crc.to_vec();
        let crc = crc16(&pkt);
        pkt.extend_from_slice(&crc.to_le_bytes());
        dev.start();
        for &b in &pkt {
            dev.write(b);
        }
        dev.stop();
        let mut out = Vec::new();
        loop {
            let b = dev.read();
            if b == 0xFF && out.len() >= 4 {
                break;
            }
            out.push(b);
            if out.len() >= 3 && out.len() == out[0] as usize {
                break;
            }
        }
        out
    }

    fn request(opcode: u8, p1: u8, p2: u16, data: &[u8]) -> Vec<u8> {
        let count = (5 + data.len() + 2) as u8;
        let mut v = vec![count, opcode, p1];
        v.extend_from_slice(&p2.to_le_bytes());
        v.extend_from_slice(data);
        v
    }

    #[test]
    fn info_returns_revision() {
        let mut d = Atecc608a::new(ADDR_DEFAULT);
        let resp = send(&mut d, &request(OP_INFO, 0, 0, &[]));
        assert_eq!(&resp[1..5], &[0x00, 0x00, 0x60, 0x03]);
        let crc = u16::from_le_bytes([resp[5], resp[6]]);
        assert_eq!(crc16(&resp[..5]), crc, "response carries a valid CRC");
    }

    #[test]
    fn random_is_deterministic_32_bytes() {
        let mut a = Atecc608a::new(ADDR_DEFAULT);
        let mut b = Atecc608a::new(ADDR_DEFAULT);
        let ra = send(&mut a, &request(OP_RANDOM, 0, 0, &[]));
        let rb = send(&mut b, &request(OP_RANDOM, 0, 0, &[]));
        assert_eq!(ra.len(), 35); // count + 32 + crc
        assert_eq!(ra, rb, "same seed → same stream");
    }

    #[test]
    fn read_slot0_returns_oem_pubkey() {
        let mut d = Atecc608a::new(ADDR_DEFAULT);
        let lo = send(&mut d, &request(OP_READ, 0x02, 0, &[]));
        let hi = send(&mut d, &request(OP_READ, 0x02, 1, &[]));
        assert_eq!(&lo[1..33], &DEFAULT_OEM_PUBKEY[..32]);
        assert_eq!(&hi[1..33], &DEFAULT_OEM_PUBKEY[32..]);
    }

    #[test]
    fn verify_accepts_real_signature_rejects_wrong_digest() {
        let mut d = Atecc608a::new(ADDR_DEFAULT);
        let digest = [0x42u8; 32];

        // Sign the digest with a throwaway key; verify with its pubkey.
        let sk = SigningKey::from_slice(&[0x11u8; 32]).unwrap();
        let sig: Signature = sk.sign_prehash(&digest).unwrap();
        let vk = sk.verifying_key().to_encoded_point(false);

        let mut d2 = Atecc608a::new(ADDR_DEFAULT);
        send(&mut d2, &request(OP_NONCE, 0x03, 0, &digest));
        let mut vd = sig.to_bytes().to_vec();
        vd.extend_from_slice(&vk.as_bytes()[1..]);
        let ok = send(&mut d2, &request(OP_VERIFY, 0x02, 0, &vd));
        assert_eq!(ok[1], STATUS_OK, "valid signature verifies");

        send(&mut d, &request(OP_NONCE, 0x03, 0, &[0x99u8; 32]));
        let mut vd2 = sig.to_bytes().to_vec();
        vd2.extend_from_slice(&vk.as_bytes()[1..]);
        let bad = send(&mut d, &request(OP_VERIFY, 0x02, 0, &vd2));
        assert_eq!(bad[1], STATUS_VERIFY_FAILED, "wrong digest fails verify");
    }

    #[test]
    fn sign_then_verify_roundtrip_with_device_key() {
        let mut d = Atecc608a::new(ADDR_DEFAULT);
        let digest = [0xABu8; 32];
        send(&mut d, &request(OP_NONCE, 0x03, 0, &digest));
        let sig_resp = send(&mut d, &request(OP_SIGN, 0, 0, &[]));
        assert_eq!(sig_resp.len(), 67);
        let sig = &sig_resp[1..65];

        // Verify with the pubkey the SE itself reports (READ slot 1).
        let lo = send(&mut d, &request(OP_READ, 0x02, 2, &[]));
        let hi = send(&mut d, &request(OP_READ, 0x02, 3, &[]));
        let mut vd = sig.to_vec();
        vd.extend_from_slice(&lo[1..33]);
        vd.extend_from_slice(&hi[1..33]);
        send(&mut d, &request(OP_NONCE, 0x03, 0, &digest));
        let ok = send(&mut d, &request(OP_VERIFY, 0x02, 0, &vd));
        assert_eq!(ok[1], STATUS_OK, "device attestation signature verifies");
    }

    #[test]
    fn bad_crc_is_rejected() {
        let mut d = Atecc608a::new(ADDR_DEFAULT);
        let mut pkt = request(OP_INFO, 0, 0, &[]);
        let crc = crc16(&pkt) ^ 0xFFFF; // corrupt
        pkt.extend_from_slice(&crc.to_le_bytes());
        d.start();
        for &b in &pkt {
            d.write(b);
        }
        let resp0 = d.read();
        let resp1 = d.read();
        assert_eq!(resp0, 4); // count
        assert_eq!(resp1, STATUS_BAD_CRC);
    }

    #[test]
    fn custom_oem_pubkey_served_from_slot0() {
        let mut custom = [0u8; 64];
        for (i, b) in custom.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(3).wrapping_add(7);
        }
        let mut dev = Atecc608a::with_oem_pubkey(ADDR_DEFAULT, custom);
        // READ slot half 0
        let mut body = vec![0u8, OP_READ, 0x02, 0x00, 0x00];
        body[0] = (body.len() + 2) as u8;
        let lo = send(&mut dev, &body);
        assert_eq!(&lo[1..33], &custom[..32]);
        let mut body = vec![0u8, OP_READ, 0x02, 0x01, 0x00];
        body[0] = (body.len() + 2) as u8;
        let hi = send(&mut dev, &body);
        assert_eq!(&hi[1..33], &custom[32..]);
    }

    #[test]
    fn parse_oem_pubkey_hex_accepts_128_digits() {
        let hex: String = (0..64).map(|i| format!("{:02x}", i)).collect();
        let pk = parse_oem_pubkey_hex(&hex).unwrap();
        assert_eq!(pk[0], 0);
        assert_eq!(pk[63], 63);
    }

    #[test]
    fn parse_oem_pubkey_hex_rejects_wrong_length() {
        assert!(parse_oem_pubkey_hex("aabb").is_err());
    }

}
