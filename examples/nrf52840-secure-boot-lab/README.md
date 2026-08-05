# nRF52840 Secure Boot + Signed OTA / Anti-Rollback Lab

A one-file demonstration that LabWired can simulate — and, more importantly,
**verify with real cryptographic evidence** — the hardware security story the
EU Cyber Resilience Act (CRA) expects from a connected device, and the one
UNECE R155/R156 expects from a vehicle ECU:

| Requirement story | What this lab shows | Sim model |
|---|---|---|
| Hardware root of trust | Boot state anchored in one-time flash (UICR), not RAM | `nrf52840_uicr` (flash 1→0 write semantics) |
| TRNG-backed key provisioning | 16-byte root key drawn from the RNG into UICR CUSTOMER | `nrf52840_rng` |
| Cryptographic boot verification | Hardware AES-128-ECB of a challenge vs a golden vector | `nrf52840_ecb` (FIPS-vector-tested AES) |
| Secure element / HSM / "external TPM" | ATECC608A on I²C: real ECDSA P-256 verify/sign, key slots that never leave the chip | `components/atecc608a.rs` (p256 crate) |
| Signed OTA update | Firmware hashes (SHA-256), the SE verifies the OEM's ECDSA signature; the private key never exists on the device | SE + UART RX injection |
| Anti-rollback (R156) | Monotonic version counter burned into one-time flash; authentic-but-old image rejected | `nrf52840_uicr` |
| Device attestation | SE signs a challenge with its internal key and verifies it — key present, usable, unextractable | SE |
| Debug lockout | `UICR.APPROTECT` programmed after provisioning | `nrf52840_uicr` |
| Reboot persistence | Real CPU reboots via `AIRCR.SYSRESETREQ`; boot N+1 verifies what boot N provisioned | sim reset machinery |

## What a single run shows

Three boots in one simulation:

1. **Factory provisioning.** UICR CUSTOMER reads erased (`0xFFFFFFFF`).
   Firmware pulls 16 bytes from the TRNG, programs them as the device root
   key into `UICR.CUSTOMER[0..4]`, burns the anti-rollback counter
   (`CUSTOMER[4]`) to v1, sets `APPROTECT`, prints `ROT: PROVISIONED`, reboots.
2. **Verified boot + signed OTA.** Hardware-encrypt a challenge, compare
   against a golden ciphertext → `SECURE BOOT OK (v1)`. A 140-byte update
   package arrives over UART (host-injected by the test script): firmware
   hashes it with software SHA-256 and the secure element verifies the OEM's
   ECDSA P-256 signature against the public key in its data slot →
   `OTA v2 SIGNATURE OK`; the payload is programmed into a flash update slot
   (authentic NVMC enable/poll sequence) and the rollback counter is burned
   forward — irreversibly, by flash semantics. Reboot.
3. **Enforcement + attestation.** Booting as v2: the committed update is read
   back from flash and hashed (`UPDATE SLOT VERIFIED`); an *authentic but
   older* v1 package is `ROLLBACK REJECTED`; a *forged* v3 package
   (corrupted signature) is `BAD SIGNATURE REJECTED` by the SE; and the SE
   signs a challenge and verifies it → `ATTESTATION OK`.

The SSD1306 OLED on `i2c0` paints every verdict (`PROVISIONING`,
`SE: SIG OK`, `ROLLBACK REJECTED`, `ATTESTATION OK`, `CRA READY`) — open the
lab in the playground and watch the whole lifecycle on the panel.

## Why the assertions are real evidence

The simulator's RNG is a deterministic xorshift32 (seed `0xC0DEF00D`), so
the provisioned key is identical on every fresh run:

```
key            = 22 CA B3 3F 02 30 A2 F2 0B EE F7 8A 31 63 B7 56
challenge      = "LabWired-ROT-v01"
boot ciphertext= F9 EE BB 62 D1 39 1E B8 BF F1 88 ED B8 24 9C 02  (openssl aes-128-ecb)
v2 pkg SHA-256 = 6F EB 7B 70 99 6C 99 C7 31 5A 35 FE 25 8E 37 39
                 E0 41 CB 9D FD 7A A1 80 25 63 3E C9 6A B5 B6 74  (openssl dgst)
```

`secure-boot-smoke.yaml` asserts those exact bytes in RAM — the boot
ciphertext produced by the simulated ECB coprocessor's real AES, the digest
the firmware computed and the SE verified against a real ECDSA signature,
and the SHA-256 of the committed update read back from flash after the
reboot — plus the UICR key readback, the one-way rollback counter
(`0xFFFFFFFC`), APPROTECT, every accept/reject verdict, and the attestation
result. Any single-bit regression in RNG, UICR, ECB, the SE model, UART RX,
or reset persistence fails the run.

## Build & run

```bash
cargo build -p firmware-nrf52840-secure-boot --target thumbv7em-none-eabi --release

cargo run -q -p labwired-cli -- test \
  --script examples/nrf52840-secure-boot-lab/secure-boot-smoke.yaml \
  --output-dir out/nrf52840-secure-boot
```

Or press **Run in LabWired** in VS Code — `.labwired/lab.yaml` is the same
script with rebased paths, so editor and CI run the identical check. The lab
is also bundled in the browser playground (nRF52840 Secure Boot).

## Files

- `system.yaml` — nRF52840 + SSD1306 OLED + ATECC608A secure element on `i2c0`.
- `secure-boot-smoke.yaml` — the 45-assertion test script, including the
  three `uart_injections` OTA packages.
- `make_packages.py` — regenerates OTA packages + digests (openssl). Use
  `--ephemeral` or `--key`; never hand-edit the byte arrays. Private keys
  are not stored in git.
- `packages.yaml` — last generated output, kept for reference.
- Firmware: `crates/firmware-nrf52840-secure-boot/` — `main.rs` lifecycle,
  `se.rs` ATECC608A driver, `sha256.rs`, `display.rs` SSD1306-over-TWIM0,
  `font5x7.rs`.
- SE model: `crates/core/src/peripherals/components/atecc608a.rs` — real
  ECDSA via the `p256` crate; ATECC608A-shaped command set
  (INFO/RANDOM/READ/NONCE/VERIFY/SIGN), CRC16-framed packets.

## Repeatability notes

- Test-observable firmware state lives in ONE `#[repr(C)] Results` struct in
  `.uninit`, which the linker places at the start of RAM: every asserted
  address is fixed by field declaration order, so firmware relayouts don't
  renumber the assertions. `.uninit` also survives the SYSRESETREQ reboots
  (cortex-m-rt doesn't re-initialize it); boot 1 zeroes it explicitly.
- The three OTA packages are injected `!at_start`; the firmware consumes one
  package per boot phase, so no cycle pacing is needed — every STARTRX finds
  its package already queued.


## CRA-style evidence pack (separate repo)

Compliance packaging, ephemeral OEM keys, and the downloadable CI artifact live in [**w1ne/labwired-cra-evidence**](https://github.com/w1ne/labwired-cra-evidence) — not in this engine tree (same split as product stacks like udslib).

```bash
git clone https://github.com/w1ne/labwired-cra-evidence
cd labwired-cra-evidence && ./scripts/run_evidence.sh
# → out/.../cra-evidence-pack/
```


## Honest limitations

- **Deterministic RNG** (both the nRF52 TRNG and the SE's RANDOM are seeded
  PRNGs). Great for reproducible CI evidence; says nothing about entropy
  quality on silicon.
- **APPROTECT is stored, not enforced.** The sim records the value; it does
  not model debug-port lockout side effects.
- **NVMC is now gated, but latency-free.** Flash stores require CONFIG.Wen
  and commit with 1→0 AND semantics, ERASEPAGE/ERASEALL blank with 0xFF,
  ERASEUICR resets the UICR — all real, all instant (READY always reads 1;
  no erase/program timing is modelled).
- **The SE model carries demo keys** and skips wake/idle timing and
  execution-time polling. The crypto is real; the command set is the
  authentic ATECC608A shape, not the full datasheet.
