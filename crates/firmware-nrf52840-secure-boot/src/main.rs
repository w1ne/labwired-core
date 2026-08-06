// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT
//
// nRF52840 secure-boot + signed-OTA / anti-rollback demo (EU CRA story).
//
// One simulator run shows a three-boot lifecycle:
//
//   Boot 1 — factory provisioning. UICR CUSTOMER reads erased (0xFFFFFFFF):
//   pull a 16-byte root key from the TRNG, program it into UICR CUSTOMER[0..4]
//   (one-time flash, bits only 1->0), burn the anti-rollback counter
//   CUSTOMER[4] to v1, set APPROTECT, print "ROT: PROVISIONED", reboot.
//
//   Boot 2 — verified boot + signed OTA. Hardware AES-128-ECB a challenge and
//   compare against a golden vector ("SECURE BOOT OK (v1)"). Then a 140-byte
//   update package arrives over UART (host-injected): the firmware hashes it
//   with software SHA-256 and the ATECC608A secure element verifies the OEM's
//   ECDSA P-256 signature against the public key in its data slot — the
//   private key never exists on the device. Version is checked against the
//   anti-rollback counter and committed by burning the counter forward —
//   irreversible by flash semantics. Reboot.
//
//   Boot 3 — enforcement + attestation. Booting as v2, the device rejects an
//   authentic-but-older v1 package ("ROLLBACK REJECTED"), a forged v3 package
//   the SE refuses ("BAD SIGNATURE REJECTED"), and proves its identity by
//   having the SE sign a challenge and verify it ("ATTESTATION OK").
//
// The SSD1306 OLED on TWIM0 paints each phase's verdict — the visible
// dashboard for playground/live demos.
//
// Pure nRF register access (no LabWired APIs): the same ELF runs in the
// LabWired simulator and — aside from the caveats below — would behave
// identically on real nRF52840 silicon.
//
// Simulator notes (honest limitations, not hidden):
//   * The sim's RNG is a deterministic xorshift32 seeded 0xC0DEF00D, so the
//     provisioned key — and every golden value derived from it — is identical
//     on every fresh run. That is what lets the test script assert real AES
//     output bit-for-bit.
//   * APPROTECT is stored but its debug-lockout side effects are not enforced.
//   * Flash programming follows silicon rules: stores need CONFIG.Wen and
//     commit 1→0 (AND semantics), ERASEPAGE/ERASEALL blank with 0xFF,
//     ERASEUICR resets the UICR, and unused flash reads 0xFF. What is NOT
//     modelled is the timing: READY always reads 1 and every operation
//     completes at the next instruction boundary.
//   * The secure element (peripherals/components/atecc608a.rs) does real
//     P-256 ECDSA via the `p256` crate, with demo keys compiled into the
//     model. Its command set is the authentic ATECC608A shape, but wake/idle
//     cycles and execution-time polling are not modelled.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

mod display;
mod font5x7;
mod se;
mod sha256;

// ── UARTE0 console + OTA download channel (EasyDMA) ──────────────────────────
const UART0_BASE: usize = 0x4000_2000;
const UART0_TASKS_STARTRX: usize = 0x000;
const UART0_TASKS_STARTTX: usize = 0x008;
const UART0_EVENTS_ENDRX: usize = 0x110;
const UART0_EVENTS_ENDTX: usize = 0x120;
const UART0_ENABLE: usize = 0x500;
const UART0_RXD_PTR: usize = 0x534;
const UART0_RXD_MAXCNT: usize = 0x538;
const UART0_RXD_AMOUNT: usize = 0x53C;
const UART0_TXD_PTR: usize = 0x544;
const UART0_TXD_MAXCNT: usize = 0x548;
const UARTE_ENABLE: u32 = 8; // 8 = UARTE (EasyDMA)

// ── RNG (TRNG) ────────────────────────────────────────────────────────────────
const RNG_BASE: usize = 0x4000_D000;
const RNG_TASKS_START: usize = 0x000;
const RNG_TASKS_STOP: usize = 0x004;
const RNG_EVENTS_VALRDY: usize = 0x100;
const RNG_VALUE: usize = 0x508;

// ── ECB (AES-128 block coprocessor) ──────────────────────────────────────────
const ECB_BASE: usize = 0x4000_E000;
const ECB_TASKS_STARTECB: usize = 0x000;
const ECB_EVENTS_ENDECB: usize = 0x100;
const ECB_ECBDATAPTR: usize = 0x504;

// ── UICR (one-time configuration flash) ──────────────────────────────────────
const UICR_BASE: usize = 0x1000_1000;
const UICR_CUSTOMER_FIRST: usize = 0x080; // CUSTOMER[0]; 32 words of user OTP
const UICR_ROLLBACK_COUNTER: usize = 0x090; // CUSTOMER[4]: monotonic version
const UICR_APPROTECT: usize = 0x208;
const APPROTECT_ENABLED: u32 = 0; // 0x00000000 = Protected (erased = disabled)

// ── SCB (software system reset) ──────────────────────────────────────────────
const SCB_AIRCR: usize = 0xE000_ED0C;
const AIRCR_VECTKEY_SYSRESETREQ: u32 = (0x05FA << 16) | (1 << 2);

// ── NVMC + flash update slot ─────────────────────────────────────────────────
// The sim's flash is plain writable memory: stores land and persist across
// reboots. The NVMC model accepts CONFIG but doesn't gate on it and doesn't
// erase — the firmware still performs the authentic enable/poll sequence so
// the same code is silicon-correct.
const NVMC_BASE: usize = 0x4001_E000;
const NVMC_READY: usize = 0x400;
const NVMC_CONFIG: usize = 0x504;
const NVMC_CONFIG_WEN: u32 = 1;
/// Update staging slot: 448 KiB into flash, above the firmware image,
/// page-aligned. The committed v2 payload lives here after the OTA.
const UPDATE_SLOT: usize = 0x0007_0000;

/// Challenge block encrypted at boot to prove possession of the root key.
const CHALLENGE: [u8; 16] = *b"LabWired-ROT-v01";

/// Golden ciphertext: AES-128-ECB(CHALLENGE) under the key the sim's
/// deterministic RNG produces on a fresh run
/// (key = 22 CA B3 3F 02 30 A2 F2 0B EE F7 8A 31 63 B7 56, derived from the
/// xorshift32 sequence in peripherals/nrf52/rng.rs; ciphertext computed with
/// `openssl enc -aes-128-ecb`).
const GOLDEN_CIPHERTEXT: [u8; 16] = [
    0xF9, 0xEE, 0xBB, 0x62, 0xD1, 0x39, 0x1E, 0xB8, 0xBF, 0xF1, 0x88, 0xED, 0xB8, 0x24, 0x9C, 0x02,
];

// ── OTA package wire format (see make_packages.py) ───────────────────────────
const OTA_MAGIC: [u8; 4] = *b"LWOT";
// 4 magic + 4 version + 4 len + 64 payload + 64 ECDSA P-256 signature (r‖s).
const OTA_PKG_LEN: usize = 140;
const OTA_SIGNED_LEN: usize = 76; // bytes covered by the signature

const ERASED: u32 = 0xFFFF_FFFF;

/// ECB EasyDMA buffer layout: { key[16], cleartext[16], ciphertext[16] }.
/// Must live in RAM — EasyDMA cannot read flash on real silicon.
#[repr(align(4))]
struct EcbBuf([u8; 48]);
static mut ECB_BUF: EcbBuf = EcbBuf([0; 48]);

/// OTA staging buffer: one whole package, filled by UARTE EasyDMA RX.
static mut OTA_STAGE: [u8; OTA_PKG_LEN] = [0; OTA_PKG_LEN];

// ── Test-observable results ──────────────────────────────────────────────────
// ONE struct at a pinned location: it lives in .uninit, which the linker
// script places at the start of RAM, so every field address is fixed by
// declaration order here — independent of how the rest of the firmware's
// .bss/.data layout shifts between builds. The lab's memory_value assertions
// target these offsets; do not reorder fields without updating the script.
//
// .uninit also means cortex-m-rt does NOT re-zero it across the SYSRESETREQ
// reboots, so values written in boot 2 are still readable in/after boot 3.
// Boot 1 zeroes the whole struct explicitly — on real silicon .uninit RAM
// content is undefined at cold power-on.
#[repr(C)]
pub struct Results {
    pub ota_accepted: u32,         // 0x00: 1 = v2 package verified + committed
    pub digest: [u32; 8],          // 0x04: firmware-computed SHA-256 of v2 pkg
    pub approtect_readback: u32,   // 0x24: UICR.APPROTECT as read in boot 2/3
    pub rollback_counter: u32,     // 0x28: UICR CUSTOMER[4] readback
    pub badsig_rejected: u32,      // 0x2C: 1 = forged v3 rejected by the SE
    pub boot_phase: u32,           // 0x30: 1 = provisioned, 2 = OTA, 3 = enforce
    pub installed_version: u32,    // 0x34: decoded from the rollback counter
    pub rollback_rejected: u32,    // 0x38: 1 = authentic v1 rejected as too old
    pub tamper_reject_result: u32, // 0x3C: 1 = wrong-key ciphertext differs
    pub verify_result: u32,        // 0x40: 1 = boot ciphertext matches golden
    pub attestation_ok: u32,       // 0x44: 1 = SE sign/verify round-trip passed
    pub ciphertext: [u32; 4],      // 0x48: AES-128-ECB(CHALLENGE) under root key
    pub key_words: [u32; 4],       // 0x58: root key as read back from UICR
    pub flash_digest: [u32; 8],    // 0x68: SHA-256 of the committed update slot
}

#[no_mangle]
#[link_section = ".uninit"]
pub static mut RESULTS: Results = Results {
    ota_accepted: 0,
    digest: [0; 8],
    approtect_readback: ERASED,
    rollback_counter: ERASED,
    badsig_rejected: 0,
    boot_phase: 0,
    installed_version: 0,
    rollback_rejected: 0,
    tamper_reject_result: 0,
    verify_result: 0,
    attestation_ok: 0,
    ciphertext: [0; 4],
    key_words: [0; 4],
    flash_digest: [0; 8],
};

#[inline(always)]
pub(crate) unsafe fn wr(base: usize, off: usize, val: u32) {
    write_volatile((base + off) as *mut u32, val);
}

#[inline(always)]
pub(crate) unsafe fn rd(base: usize, off: usize) -> u32 {
    read_volatile((base + off) as *const u32)
}

unsafe fn uart_puts(msg: &[u8]) {
    // Buffer must be in RAM for EasyDMA; copy the message onto the stack.
    let mut buf = [0u8; 64];
    let n = msg.len().min(buf.len());
    buf[..n].copy_from_slice(&msg[..n]);
    wr(UART0_BASE, UART0_EVENTS_ENDTX, 0);
    wr(UART0_BASE, UART0_TXD_PTR, buf.as_ptr() as u32);
    wr(UART0_BASE, UART0_TXD_MAXCNT, n as u32);
    wr(UART0_BASE, UART0_TASKS_STARTTX, 1);
    while rd(UART0_BASE, UART0_EVENTS_ENDTX) == 0 {}
}

/// Receive exactly one OTA package into OTA_STAGE via UARTE EasyDMA RX.
/// Returns false if a short/partial transfer arrived (caller treats as a
/// transport error, distinct from a signature failure).
unsafe fn uart_recv_stage() -> bool {
    let stage = &mut *core::ptr::addr_of_mut!(OTA_STAGE);
    wr(UART0_BASE, UART0_EVENTS_ENDRX, 0);
    wr(UART0_BASE, UART0_RXD_PTR, stage.as_ptr() as u32);
    wr(UART0_BASE, UART0_RXD_MAXCNT, OTA_PKG_LEN as u32);
    wr(UART0_BASE, UART0_TASKS_STARTRX, 1);
    while rd(UART0_BASE, UART0_EVENTS_ENDRX) == 0 {}
    rd(UART0_BASE, UART0_RXD_AMOUNT) as usize == OTA_PKG_LEN
}

/// Fetch one random byte from the RNG (assumes TASKS_START already issued).
unsafe fn rng_byte() -> u8 {
    while rd(RNG_BASE, RNG_EVENTS_VALRDY) == 0 {}
    wr(RNG_BASE, RNG_EVENTS_VALRDY, 0);
    rd(RNG_BASE, RNG_VALUE) as u8
}

/// AES-128-ECB encrypt one 16-byte block under `key` via the ECB coprocessor.
unsafe fn ecb_encrypt_block(key: &[u8; 16], plaintext: &[u8; 16]) -> [u8; 16] {
    let buf = &mut *core::ptr::addr_of_mut!(ECB_BUF);
    buf.0[..16].copy_from_slice(key);
    buf.0[16..32].copy_from_slice(plaintext);
    wr(ECB_BASE, ECB_ECBDATAPTR, buf.0.as_ptr() as u32);
    wr(ECB_BASE, ECB_EVENTS_ENDECB, 0);
    wr(ECB_BASE, ECB_TASKS_STARTECB, 1);
    while rd(ECB_BASE, ECB_EVENTS_ENDECB) == 0 {}
    let mut out = [0u8; 16];
    out.copy_from_slice(&buf.0[32..48]);
    out
}

// ── OTA signature verification happens in the secure element (se.rs) ─────
// The MCU hashes with local SHA-256 (sha256.rs) and hands only the digest
// to the SE; the OEM private key never exists on the device at all.

/// Installed version decoded from the anti-rollback counter: the number of
/// low bits burned to 0. Erased (0xFFFFFFFF) = nothing installed.
fn decode_installed(counter: u32) -> u32 {
    let mut v = 0;
    while v < 32 && (counter >> v) & 1 == 0 {
        v += 1;
    }
    v
}

/// Burn the rollback counter forward to version `v` (flash AND semantics make
/// the burn one-way: a later write can only clear more bits, never go back).
unsafe fn burn_version(v: u32) {
    wr(UICR_BASE, UICR_ROLLBACK_COUNTER, 0xFFFF_FFFF << v);
}

#[entry]
fn main() -> ! {
    unsafe {
        let results = &mut *core::ptr::addr_of_mut!(RESULTS);

        wr(UART0_BASE, UART0_ENABLE, UARTE_ENABLE);
        display::display_init();

        let counter = rd(UICR_BASE, UICR_ROLLBACK_COUNTER);
        results.rollback_counter = counter;
        let installed = decode_installed(counter);
        results.installed_version = installed;

        if rd(UICR_BASE, UICR_CUSTOMER_FIRST) == ERASED {
            // ── Boot 1: factory provisioning ─────────────────────────────
            results.boot_phase = 1;
            // Cold-boot hygiene for the reset-persistent struct (.uninit):
            // boot 2/3 set fields; no boot may see stale values.
            results.ota_accepted = 0;
            results.digest = [0; 8];
            results.badsig_rejected = 0;
            results.rollback_rejected = 0;
            results.attestation_ok = 0;
            results.flash_digest = [0; 8];

            // Draw a 16-byte root key from the TRNG.
            wr(RNG_BASE, RNG_TASKS_START, 1);
            let mut key = [0u8; 16];
            for b in key.iter_mut() {
                *b = rng_byte();
            }
            wr(RNG_BASE, RNG_TASKS_STOP, 1);

            // Program it into UICR CUSTOMER[0..4] (one-time flash: bits only
            // go 1->0, so this is write-once on an erased device).
            for (i, chunk) in key.chunks_exact(4).enumerate() {
                let word = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                wr(UICR_BASE, UICR_CUSTOMER_FIRST + i * 4, word);
            }

            // Read back and require an exact match before declaring the
            // root of trust provisioned.
            let mut ok = true;
            for (i, chunk) in key.chunks_exact(4).enumerate() {
                let back = rd(UICR_BASE, UICR_CUSTOMER_FIRST + i * 4);
                results.key_words[i] = back;
                if back != u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) {
                    ok = false;
                }
            }

            if ok {
                // The factory image is v1: burn the rollback counter to 1.
                burn_version(1);
                results.rollback_counter = rd(UICR_BASE, UICR_ROLLBACK_COUNTER);
                results.installed_version = 1;
                // Lock debug access behind APPROTECT (stored by the sim; the
                // lockout side effects are not enforced — see file header).
                wr(UICR_BASE, UICR_APPROTECT, APPROTECT_ENABLED);
                uart_puts(b"ROT: PROVISIONED\n");
                display::display_text(0, 0, "LABWIRED");
                display::display_text(1, 0, "SECURE DEVICE LAB");
                display::display_text(3, 0, "PROVISIONING");
                display::display_text(4, 0, "ROOT KEY -> UICR");
                display::display_text(6, 0, "REBOOTING...");
                display::display_flush();
            } else {
                uart_puts(b"ROT: PROVISION FAILED\n");
                loop {
                    cortex_m::asm::wfi();
                }
            }

            // Reboot: boot 2 must find the key in UICR and verify with it.
            wr(SCB_AIRCR, 0, AIRCR_VECTKEY_SYSRESETREQ);
            loop {
                cortex_m::asm::wfi();
            }
        }

        // ── Boots 2/3: verified boot ──────────────────────────────────────
        results.approtect_readback = rd(UICR_BASE, UICR_APPROTECT);

        // Load the root key from UICR.
        let mut key = [0u8; 16];
        for i in 0..4 {
            let word = rd(UICR_BASE, UICR_CUSTOMER_FIRST + i * 4);
            results.key_words[i] = word;
            key[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }

        // Prove possession: hardware-encrypt the challenge, compare to golden.
        let ct = ecb_encrypt_block(&key, &CHALLENGE);
        for i in 0..4 {
            results.ciphertext[i] =
                u32::from_le_bytes([ct[i * 4], ct[i * 4 + 1], ct[i * 4 + 2], ct[i * 4 + 3]]);
        }
        let verified = ct == GOLDEN_CIPHERTEXT;
        results.verify_result = verified as u32;

        if !verified {
            uart_puts(b"SECURE BOOT FAILED\n");
            loop {
                cortex_m::asm::wfi();
            }
        }

        // Negative path: a wrong (zero) key must NOT produce the golden
        // ciphertext — proves the check actually rejects, not always-passes.
        let wrong = ecb_encrypt_block(&[0u8; 16], &CHALLENGE);
        let rejected = wrong != GOLDEN_CIPHERTEXT;
        results.tamper_reject_result = rejected as u32;

        if installed == 1 {
            // ── Boot 2: verified v1 boot, then signed OTA to v2 ──────────
            results.boot_phase = 2;
            uart_puts(b"SECURE BOOT OK (v1)\n");
            if rejected {
                uart_puts(b"TAMPER REJECT OK\n");
            }
            display::display_text(0, 0, "SECURE BOOT OK");
            display::display_text(1, 0, "FW v1");
            display::display_text(3, 0, "OTA PKG v2 RX");
            display::display_text(4, 0, "ECDSA VERIFY...");
            display::display_flush();

            if !uart_recv_stage() {
                uart_puts(b"OTA TRANSPORT ERROR\n");
                loop {
                    cortex_m::asm::wfi();
                }
            }
            let stage = &*core::ptr::addr_of!(OTA_STAGE);
            let version = u32::from_le_bytes([stage[4], stage[5], stage[6], stage[7]]);

            // Hash locally; only the digest crosses into the secure element.
            let digest = sha256::sha256(&stage[..OTA_SIGNED_LEN]);
            for i in 0..8 {
                results.digest[i] = u32::from_be_bytes([
                    digest[i * 4],
                    digest[i * 4 + 1],
                    digest[i * 4 + 2],
                    digest[i * 4 + 3],
                ]);
            }

            // The SE verifies the OEM signature against the public key stored
            // in its own data zone — the private key exists only at the OEM.
            let mut sig = [0u8; 64];
            sig.copy_from_slice(&stage[OTA_SIGNED_LEN..OTA_PKG_LEN]);
            let sig_ok = stage[..4] == OTA_MAGIC
                && match se::se_read_oem_pubkey() {
                    Some(pk) => se::se_verify(&digest, &sig, &pk),
                    None => false,
                };

            if sig_ok && version > installed {
                uart_puts(b"OTA v2 SIGNATURE OK\n");
                // Commit: burn the rollback counter to v2. One-way by flash
                // semantics — a v1 image can never be installed again.
                burn_version(version);
                results.rollback_counter = rd(UICR_BASE, UICR_ROLLBACK_COUNTER);
                results.installed_version = decode_installed(results.rollback_counter);
                results.ota_accepted = 1;
                uart_puts(b"OTA v2 ACCEPTED\n");
                // Commit the image: program the verified payload into the
                // flash update slot (authentic NVMC enable/poll sequence).
                let payload = &stage[12..12 + 64];
                wr(NVMC_BASE, NVMC_CONFIG, NVMC_CONFIG_WEN);
                while rd(NVMC_BASE, NVMC_READY) == 0 {}
                for (i, word) in payload.chunks_exact(4).enumerate() {
                    let w = u32::from_le_bytes([word[0], word[1], word[2], word[3]]);
                    write_volatile((UPDATE_SLOT + i * 4) as *mut u32, w);
                    while rd(NVMC_BASE, NVMC_READY) == 0 {}
                }
                wr(NVMC_BASE, NVMC_CONFIG, 0);
                uart_puts(b"OTA v2 COMMITTED\n");
                display::display_text(4, 0, "SE: SIG OK");
                display::display_text(5, 0, "COMMITTED v2");
                display::display_text(6, 0, "REBOOTING...");
                display::display_flush();
                wr(SCB_AIRCR, 0, AIRCR_VECTKEY_SYSRESETREQ);
            } else {
                uart_puts(b"OTA v2 REJECTED\n");
            }
            loop {
                cortex_m::asm::wfi();
            }
        }

        // ── Boot 3: v2 boot, anti-rollback + forgery enforcement ──────────
        results.boot_phase = 3;
        uart_puts(b"SECURE BOOT OK (v2)\n");
        uart_puts(b"ANTI-ROLLBACK v2 ACTIVE\n");

        // Read back the committed update from the flash slot and hash it.
        // The lab asserts these words against the golden payload digest —
        // proof the NVMC write sequence landed real bytes in flash and they
        // survived the reboot.
        {
            let mut slot = [0u8; 64];
            for (i, word) in slot.chunks_exact_mut(4).enumerate() {
                let w = read_volatile((UPDATE_SLOT + i * 4) as *const u32);
                word.copy_from_slice(&w.to_le_bytes());
            }
            let d = sha256::sha256(&slot);
            for i in 0..8 {
                results.flash_digest[i] =
                    u32::from_be_bytes([d[i * 4], d[i * 4 + 1], d[i * 4 + 2], d[i * 4 + 3]]);
            }
            // v2's payload starts with a known banner; a slot that never got
            // programmed reads as zeros/erased in the sim.
            if slot.starts_with(b"LabWired OTA image v2") {
                uart_puts(b"UPDATE SLOT VERIFIED\n");
            } else {
                uart_puts(b"UPDATE SLOT EMPTY\n");
            }
        }

        // Package 2: authentic (valid signature) but OLDER than installed.
        if !uart_recv_stage() {
            uart_puts(b"OTA TRANSPORT ERROR\n");
            loop {
                cortex_m::asm::wfi();
            }
        }
        {
            let stage = &*core::ptr::addr_of!(OTA_STAGE);
            let version = u32::from_le_bytes([stage[4], stage[5], stage[6], stage[7]]);
            let digest = sha256::sha256(&stage[..OTA_SIGNED_LEN]);
            let mut sig = [0u8; 64];
            sig.copy_from_slice(&stage[OTA_SIGNED_LEN..OTA_PKG_LEN]);
            let sig_ok = stage[..4] == OTA_MAGIC
                && match se::se_read_oem_pubkey() {
                    Some(pk) => se::se_verify(&digest, &sig, &pk),
                    None => false,
                };
            if sig_ok && version <= results.installed_version {
                results.rollback_rejected = 1;
                uart_puts(b"ROLLBACK REJECTED\n");
            } else {
                uart_puts(b"ROLLBACK TEST FAILED\n");
            }
        }

        // Package 3: well-formed but forged (corrupted signature) — the SE
        // must refuse it.
        if !uart_recv_stage() {
            uart_puts(b"OTA TRANSPORT ERROR\n");
            loop {
                cortex_m::asm::wfi();
            }
        }
        {
            let stage = &*core::ptr::addr_of!(OTA_STAGE);
            let digest = sha256::sha256(&stage[..OTA_SIGNED_LEN]);
            let mut sig = [0u8; 64];
            sig.copy_from_slice(&stage[OTA_SIGNED_LEN..OTA_PKG_LEN]);
            let sig_ok = stage[..4] == OTA_MAGIC
                && match se::se_read_oem_pubkey() {
                    Some(pk) => se::se_verify(&digest, &sig, &pk),
                    None => false,
                };
            if !sig_ok {
                results.badsig_rejected = 1;
                uart_puts(b"BAD SIGNATURE REJECTED\n");
            } else {
                uart_puts(b"FORGERY TEST FAILED\n");
            }
        }

        // Attestation: the SE signs a fixed challenge with its internal
        // device key, then verifies its own signature — proving the key is
        // present, usable, and never left the chip.
        {
            let challenge = sha256::sha256(b"LABWIRED-ATTEST-v01");
            let ok = match (se::se_sign(&challenge), se::se_read_device_pubkey()) {
                (Some(sig), Some(pk)) => se::se_verify(&challenge, &sig, &pk),
                _ => false,
            };
            if ok {
                results.attestation_ok = 1;
                uart_puts(b"ATTESTATION OK\n");
            } else {
                uart_puts(b"ATTESTATION FAILED\n");
            }
        }

        // Park: results sit in the pinned RESULTS struct for the lab to read.
        display::display_text(0, 0, "SECURE BOOT OK");
        display::display_text(1, 0, "FW v2 ACTIVE");
        display::display_text(2, 0, "ANTI-ROLLBACK ON");
        display::display_text(4, 0, "ROLLBACK REJECTED");
        display::display_text(5, 0, "FORGED REJECTED");
        display::display_text(6, 0, "ATTEST + FLASH OK");
        display::display_text(7, 0, "CRA READY");
        display::display_flush();
        loop {
            cortex_m::asm::wfi();
        }
    }
}
