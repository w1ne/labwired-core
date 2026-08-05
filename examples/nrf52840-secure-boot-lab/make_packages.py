#!/usr/bin/env python3
# LabWired - Firmware Simulation Platform
# SPDX-License-Identifier: MIT
"""Generate signed OTA demo packages for the nrf52840-secure-boot-lab.

The OEM *private* key is never stored in this repository. Supply one via:

  --ephemeral   generate a throwaway P-256 keypair (CI default)
  --key PATH    use an existing PEM private key (local only)

Writes (into --out-dir, default stdout for yaml + digests to stdout notes):

  packages.yaml          uart_injections fragment
  oem-verify-pubkey.hex  64-byte OEM public key (X‖Y) as 128 hex chars
  digests.json           golden SHA-256 digests for smoke assertions
  oem-private.pem        only with --ephemeral if --keep-private is set
                         (default: private key stays in a temp dir and is deleted)

Package format (140 bytes):
    magic[4]      "LWOT"
    version u32 LE
    payload_len u32 LE (64)
    payload[64]
    sig[64]       ECDSA P-256 (r||s) over SHA-256 of the first 76 bytes

Requires openssl 3.x on PATH.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
import tempfile
from pathlib import Path


def payload(text: str) -> bytes:
    b = text.encode()
    assert len(b) <= 64, "payload too long"
    return b.ljust(64, b"\0")


def header(version: int, body: bytes) -> bytes:
    return b"LWOT" + version.to_bytes(4, "little") + len(body).to_bytes(4, "little") + body


def der_to_raw64(der: bytes) -> bytes:
    """Minimal DER INTEGER×2 parse → raw P1363 r||s (64 bytes)."""
    assert der[0] == 0x30
    i = 2
    if der[1] & 0x80:
        nlen = der[1] & 0x7F
        i = 2 + nlen
    ints = []
    for _ in range(2):
        assert der[i] == 0x02
        ln = der[i + 1]
        v = der[i + 2 : i + 2 + ln]
        ints.append(int.from_bytes(v, "big"))
        i += 2 + ln
    return ints[0].to_bytes(32, "big") + ints[1].to_bytes(32, "big")


def sign(data: bytes, key_path: Path) -> bytes:
    out = subprocess.run(
        ["openssl", "dgst", "-sha256", "-sign", str(key_path)],
        input=data,
        capture_output=True,
        check=True,
    )
    return der_to_raw64(out.stdout)


def package(version: int, text: str, key_path: Path, corrupt_sig: bool = False) -> bytes:
    head = header(version, payload(text))
    sig = bytearray(sign(head, key_path))
    if corrupt_sig:
        sig[0] ^= 0xFF
    return head + bytes(sig)


def yaml_bytes(data: bytes) -> str:
    rows = []
    for i in range(0, len(data), 16):
        rows.append(", ".join(f"0x{b:02X}" for b in data[i : i + 16]))
    return ",\n        ".join(rows)


def be_words(data: bytes) -> list[str]:
    return [f"0x{int.from_bytes(data[i : i + 4], 'big'):08X}" for i in range(0, len(data), 4)]


def pubkey_xy_from_private(key_path: Path) -> bytes:
    """Return uncompressed P-256 public key without 0x04 prefix (64 bytes)."""
    out = subprocess.run(
        ["openssl", "ec", "-in", str(key_path), "-pubout", "-outform", "DER"],
        capture_output=True,
        check=True,
    )
    der = out.stdout
    # SubjectPublicKeyInfo … BIT STRING of 0x04 || X || Y (65 bytes raw).
    # Find the 0x04 that starts the uncompressed point near the end.
    idx = der.rfind(b"\x04")
    if idx < 0 or len(der) - idx < 65:
        raise RuntimeError("could not locate uncompressed EC point in DER pubkey")
    point = der[idx : idx + 65]
    if point[0] != 0x04:
        raise RuntimeError("expected uncompressed EC point")
    return point[1:65]


def generate_ephemeral_key(path: Path) -> None:
    subprocess.run(
        ["openssl", "ecparam", "-name", "prime256v1", "-genkey", "-noout", "-out", str(path)],
        check=True,
        capture_output=True,
    )


def build_packages(key_path: Path) -> list[tuple[str, bytes]]:
    return [
        (
            "v2-accept",
            package(2, "LabWired OTA image v2 (demo payload, not executable)", key_path),
        ),
        (
            "v1-rollback",
            package(1, "LabWired OTA image v1 - authentic but OLD", key_path),
        ),
        (
            "v3-forged",
            package(3, "LabWired OTA image v3 - FORGED signature", key_path, corrupt_sig=True),
        ),
    ]


def write_packages_yaml(pkgs: list[tuple[str, bytes]], path: Path) -> None:
    lines = ["uart_injections:"]
    for name, pkg in pkgs:
        lines.append(f"  # ── {name} ({len(pkg)} bytes) ──")
        lines.append('  - uart: "uart0"')
        lines.append("    trigger: !at_start")
        lines.append("    bytes:")
        lines.append(f"      [{yaml_bytes(pkg)}]")
    path.write_text("\n".join(lines) + "\n")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    g = ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--ephemeral", action="store_true", help="generate a throwaway P-256 key")
    g.add_argument("--key", type=Path, help="path to OEM private key PEM")
    ap.add_argument(
        "--out-dir",
        type=Path,
        default=Path("."),
        help="directory for packages.yaml, digests.json, oem-verify-pubkey.hex",
    )
    ap.add_argument(
        "--keep-private",
        action="store_true",
        help="with --ephemeral, also write oem-private.pem into --out-dir (dev only)",
    )
    args = ap.parse_args()
    args.out_dir.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory() as td:
        td_path = Path(td)
        if args.ephemeral:
            key_path = td_path / "oem-private.pem"
            generate_ephemeral_key(key_path)
            if args.keep_private:
                (args.out_dir / "oem-private.pem").write_bytes(key_path.read_bytes())
        else:
            key_path = args.key
            if not key_path.is_file():
                print(f"error: key file not found: {key_path}", file=sys.stderr)
                return 2

        pkgs = build_packages(key_path)
        pub = pubkey_xy_from_private(key_path)
        pub_hex = pub.hex()

        write_packages_yaml(pkgs, args.out_dir / "packages.yaml")
        (args.out_dir / "oem-verify-pubkey.hex").write_text(pub_hex + "\n")

        v2 = pkgs[0][1]
        header_digest = hashlib.sha256(v2[:76]).digest()
        payload_digest = hashlib.sha256(v2[12:76]).digest()
        digests = {
            "v2_header_payload_sha256_hex": header_digest.hex(),
            "v2_header_payload_sha256_words_be": be_words(header_digest),
            "v2_payload_sha256_hex": payload_digest.hex(),
            "v2_payload_sha256_words_be": be_words(payload_digest),
            "oem_pubkey_hex": pub_hex,
        }
        (args.out_dir / "digests.json").write_text(json.dumps(digests, indent=2) + "\n")

        # Human-readable summary on stdout
        print(f"wrote {args.out_dir / 'packages.yaml'}")
        print(f"wrote {args.out_dir / 'oem-verify-pubkey.hex'} ({pub_hex[:16]}…)")
        print(f"wrote {args.out_dir / 'digests.json'}")
        print(f"# v2 header+payload SHA-256: {header_digest.hex()}")
        print(f"# v2 payload SHA-256:        {payload_digest.hex()}")
        if args.ephemeral and not args.keep_private:
            print("# ephemeral private key discarded with temp dir")
    return 0


if __name__ == "__main__":
    sys.exit(main())
