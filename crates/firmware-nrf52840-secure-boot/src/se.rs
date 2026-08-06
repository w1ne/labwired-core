// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT
//
// ATECC608A secure-element driver over TWIM0 (shared with the OLED).
//
// Packet format (mirrors the sim model, which mirrors the real part):
//   request:  [count, opcode, p1, p2le, ...data, crc16le]
//   response: [count, ...data, crc16le]
// CRC16-CCITT (poly 0x8005, reflected, init 0) over count..last-data.
//
// Every SE transaction is write-then-read: the TWIM SHORTS LASTTX→STARTRX
// short chains the RX after the TX. SHORTS is set per transaction and
// cleared afterwards so plain TX users of the same bus (the OLED) are
// unaffected.

use crate::display;
use crate::{rd, wr};

const TWIM0_BASE: usize = display::TWIM0_BASE;
const TWIM_TASKS_STARTTX: usize = 0x008;
const TWIM_EVENTS_LASTRX: usize = 0x15C;
const TWIM_SHORTS: usize = 0x200;
const TWIM_RXD_PTR: usize = 0x534;
const TWIM_RXD_MAXCNT: usize = 0x538;
const TWIM_TXD_PTR: usize = 0x544;
const TWIM_TXD_MAXCNT: usize = 0x548;
const TWIM_ADDRESS: usize = 0x588;
const SHORT_LASTTX_STARTRX: u32 = 1 << 7;

const SE_ADDR: u32 = 0x60;

const OP_NONCE: u8 = 0x16;
const OP_READ: u8 = 0x02;
const OP_SIGN: u8 = 0x41;
const OP_VERIFY: u8 = 0x45;

const STATUS_OK: u8 = 0x00;

fn crc16(bytes: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &byte in bytes {
        crc ^= byte as u16;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xA001
            } else {
                crc >> 1
            };
        }
    }
    crc
}

/// One SE round-trip. `req_body` is [count..data] WITHOUT the trailing CRC
/// (appended here). Returns the response data length written into `resp`
/// (excluding count and CRC), or 0 on transport/CRC failure.
unsafe fn se_transact(req_body: &[u8], resp: &mut [u8]) -> usize {
    let mut pkt = [0u8; 160];
    pkt[..req_body.len()].copy_from_slice(req_body);
    let crc = crc16(&pkt[..req_body.len()]);
    pkt[req_body.len()] = crc as u8;
    pkt[req_body.len() + 1] = (crc >> 8) as u8;
    let pkt_len = req_body.len() + 2;

    wr(TWIM0_BASE, TWIM_ADDRESS, SE_ADDR);
    wr(TWIM0_BASE, TWIM_EVENTS_LASTRX, 0);
    wr(TWIM0_BASE, TWIM_RXD_PTR, resp.as_mut_ptr() as u32);
    wr(TWIM0_BASE, TWIM_RXD_MAXCNT, resp.len() as u32);
    wr(TWIM0_BASE, TWIM_TXD_PTR, pkt.as_ptr() as u32);
    wr(TWIM0_BASE, TWIM_TXD_MAXCNT, pkt_len as u32);
    wr(TWIM0_BASE, TWIM_SHORTS, SHORT_LASTTX_STARTRX);
    wr(TWIM0_BASE, TWIM_TASKS_STARTTX, 1);
    while rd(TWIM0_BASE, TWIM_EVENTS_LASTRX) == 0 {}
    wr(TWIM0_BASE, TWIM_SHORTS, 0);

    // Response: [count, data..., crc16le]. Validate framing + CRC.
    let count = resp[0] as usize;
    if count < 4 || count > resp.len() {
        return 0;
    }
    let want = u16::from_le_bytes([resp[count - 2], resp[count - 1]]);
    if crc16(&resp[..count - 2]) != want {
        return 0;
    }
    count - 3
}

fn request(opcode: u8, p1: u8, p2: u16, data: &[u8], out: &mut [u8; 160]) -> usize {
    let count = (5 + data.len() + 2) as u8;
    out[0] = count;
    out[1] = opcode;
    out[2] = p1;
    out[3..5].copy_from_slice(&p2.to_le_bytes());
    out[5..5 + data.len()].copy_from_slice(data);
    5 + data.len()
}

/// Read one 32-byte half of a data-zone slot (slot*2 + half).
unsafe fn se_read_slot_half(slot_half: u16, out: &mut [u8; 32]) -> bool {
    let mut req = [0u8; 160];
    let n = request(OP_READ, 0x02, slot_half, &[], &mut req);
    let mut resp = [0u8; 40];
    let got = se_transact(&req[..n], &mut resp);
    if got != 32 {
        return false;
    }
    out.copy_from_slice(&resp[1..33]);
    true
}

/// The OEM update-verify public key from data slot 0 (64 bytes X‖Y).
pub unsafe fn se_read_oem_pubkey() -> Option<[u8; 64]> {
    let mut pk = [0u8; 64];
    let mut half = [0u8; 32];
    if !se_read_slot_half(0, &mut half) {
        return None;
    }
    pk[..32].copy_from_slice(&half);
    if !se_read_slot_half(1, &mut half) {
        return None;
    }
    pk[32..].copy_from_slice(&half);
    Some(pk)
}

/// The device attestation public key (slot 1, 64 bytes X‖Y).
pub unsafe fn se_read_device_pubkey() -> Option<[u8; 64]> {
    let mut pk = [0u8; 64];
    let mut half = [0u8; 32];
    if !se_read_slot_half(2, &mut half) {
        return None;
    }
    pk[..32].copy_from_slice(&half);
    if !se_read_slot_half(3, &mut half) {
        return None;
    }
    pk[32..].copy_from_slice(&half);
    Some(pk)
}

/// NONCE passthrough: load a 32-byte digest into the SE's tempkey.
unsafe fn se_nonce(digest: &[u8; 32]) -> bool {
    let mut req = [0u8; 160];
    let n = request(OP_NONCE, 0x03, 0, digest, &mut req);
    let mut resp = [0u8; 8];
    se_transact(&req[..n], &mut resp) == 1 && resp[1] == STATUS_OK
}

/// VERIFY external: ECDSA P-256 verify of `sig` (r‖s, 64B) over `digest`
/// with `pubkey` (X‖Y, 64B). The digest lives only in the SE's tempkey —
/// the private key never touches the main MCU.
pub unsafe fn se_verify(digest: &[u8; 32], sig: &[u8; 64], pubkey: &[u8; 64]) -> bool {
    if !se_nonce(digest) {
        return false;
    }
    let mut data = [0u8; 128];
    data[..64].copy_from_slice(sig);
    data[64..].copy_from_slice(pubkey);
    let mut req = [0u8; 160];
    let n = request(OP_VERIFY, 0x02, 0, &data, &mut req);
    let mut resp = [0u8; 8];
    se_transact(&req[..n], &mut resp) == 1 && resp[1] == STATUS_OK
}

/// SIGN: the SE signs `digest` with its internal device key (never
/// extractable). Returns the 64-byte r‖s attestation signature.
pub unsafe fn se_sign(digest: &[u8; 32]) -> Option<[u8; 64]> {
    if !se_nonce(digest) {
        return None;
    }
    let mut req = [0u8; 160];
    let n = request(OP_SIGN, 0, 0, &[], &mut req);
    let mut resp = [0u8; 72];
    if se_transact(&req[..n], &mut resp) != 64 {
        return None;
    }
    let mut sig = [0u8; 64];
    sig.copy_from_slice(&resp[1..65]);
    Some(sig)
}
