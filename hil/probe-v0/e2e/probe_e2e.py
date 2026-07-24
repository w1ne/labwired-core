#!/usr/bin/env python3
"""End-to-end test for the Probe V0 compute link (Orange Pi -> /dev/spidev1.0).

This is the first real e2e gate from the risk-first spec (see ../design.md,
Stage 1 "compute side"): prove that a controlling host can reach the Orange Pi
and drive the SPI1 master that the GW1N FPGA will hang off of.

It is a *hardware* test — it talks to a live Orange Pi over SSH and exercises the
kernel SPI master through spidev. With nothing wired to MISO yet it asserts the
path works (device present, ioctl transfer succeeds, correct byte count); once
the FPGA is wired, `--expect-loopback` / the device-id check tighten the bar.

Configuration is entirely via environment so no host/credentials live in the repo:

    PROBE_PI_HOST      required, e.g. "root@192.168.1.247"
    PROBE_SSH_KEY      optional, path to the private key (default: ssh's own resolution)
    PROBE_SSH_JUMP     optional, a ProxyCommand string. For the mesh-isolated lab
                       setup this jumps through the router, e.g.:
                         sshpass -p PW ssh -o PreferredAuthentications=password \\
                           -o PubkeyAuthentication=no root@192.168.1.1 nc %h %p
    PROBE_SPI_BUS      optional, default 1   (SPI1 on the 26-pin header)
    PROBE_SPI_CS       optional, default 0   (-> /dev/spidev1.0)
    PROBE_SPI_HZ       optional, default 1000000

Run standalone (prints a report, exit 0/1):

    PROBE_PI_HOST=root@192.168.1.247 python3 probe_e2e.py

Or under pytest (skips cleanly when PROBE_PI_HOST is unset):

    pytest probe_e2e.py -v
"""
from __future__ import annotations

import os
import shlex
import subprocess
import sys

SPI_BUS = int(os.environ.get("PROBE_SPI_BUS", "1"))
SPI_CS = int(os.environ.get("PROBE_SPI_CS", "0"))
SPI_HZ = int(os.environ.get("PROBE_SPI_HZ", "1000000"))
SPIDEV_NODE = f"/dev/spidev{SPI_BUS}.{SPI_CS}"


def _ssh_base() -> list[str]:
    host = os.environ.get("PROBE_PI_HOST")
    if not host:
        raise RuntimeError("PROBE_PI_HOST is not set (e.g. root@192.168.1.247)")
    cmd = [
        "ssh",
        "-o", "StrictHostKeyChecking=no",
        "-o", "UserKnownHostsFile=/dev/null",
        "-o", "ConnectTimeout=12",
        "-o", "BatchMode=yes",
    ]
    key = os.environ.get("PROBE_SSH_KEY")
    if key:
        cmd += ["-i", key]
    jump = os.environ.get("PROBE_SSH_JUMP")
    if jump:
        cmd += ["-o", f"ProxyCommand={jump}"]
    cmd.append(host)
    return cmd


def run_remote(remote_cmd: str, timeout: int = 40) -> subprocess.CompletedProcess:
    """Run a command on the Pi, returning the CompletedProcess (stderr filtered)."""
    proc = subprocess.run(
        _ssh_base() + [remote_cmd],
        capture_output=True,
        text=True,
        timeout=timeout,
    )
    # ssh prints the known-hosts warning to stderr; drop it so callers see real errors.
    proc.stderr = "\n".join(
        ln for ln in proc.stderr.splitlines()
        if "Warning: Permanently added" not in ln
    )
    return proc


# --- individual checks -------------------------------------------------------

def check_reachable() -> str:
    p = run_remote("hostname")
    assert p.returncode == 0, f"SSH to Pi failed: {p.stderr.strip()}"
    return p.stdout.strip()


def check_spidev_present() -> None:
    p = run_remote(f"test -c {SPIDEV_NODE} && echo OK")
    assert "OK" in p.stdout, (
        f"{SPIDEV_NODE} not present. Ensure armbianEnv.txt has "
        f"overlays=spidev{SPI_BUS}_{SPI_CS} AND overlay_prefix=sun50i-h616."
    )


def check_spi_master() -> list[str]:
    p = run_remote("ls /sys/class/spi_master/ 2>/dev/null")
    masters = p.stdout.split()
    assert f"spi{SPI_BUS}" in masters, (
        f"spi{SPI_BUS} master missing; found {masters or 'none'}"
    )
    return masters


def check_spi_transfer() -> list[int]:
    """Clock 4 bytes through the master and assert the ioctl path succeeds."""
    tx = [0xAA, 0x55, 0x00, 0xFF]
    py = (
        "import spidev,json,sys;"
        "s=spidev.SpiDev();"
        f"s.open({SPI_BUS},{SPI_CS});"
        f"s.max_speed_hz={SPI_HZ};s.mode=0;"
        f"r=s.xfer2({tx});"
        "s.close();"
        "print(json.dumps(r))"
    )
    # Ensure the module is present (idempotent; no-op once installed).
    run_remote("dpkg -s python3-spidev >/dev/null 2>&1 || "
               "apt-get install -y python3-spidev >/dev/null 2>&1", timeout=120)
    p = run_remote(f"python3 -c {shlex.quote(py)}")
    assert p.returncode == 0, f"spidev transfer failed: {p.stderr.strip() or p.stdout.strip()}"
    import json
    rx = json.loads(p.stdout.strip().splitlines()[-1])
    assert len(rx) == len(tx), f"expected {len(tx)} bytes back, got {rx}"

    if os.environ.get("PROBE_EXPECT_LOOPBACK") == "1":
        # MOSI jumpered to MISO: the master must read back exactly what it sent.
        assert rx == tx, f"loopback mismatch: sent {tx}, read {rx}"
    return rx


CHECKS = [
    ("pi reachable over ssh", check_reachable),
    (f"{SPIDEV_NODE} present", check_spidev_present),
    (f"spi{SPI_BUS} master registered", check_spi_master),
    ("spi ioctl transfer round-trips", check_spi_transfer),
]


def main() -> int:
    if not os.environ.get("PROBE_PI_HOST"):
        print("SKIP: PROBE_PI_HOST not set — this is a hardware e2e test.")
        return 0
    print(f"Probe V0 compute-link e2e  (bus={SPI_BUS} cs={SPI_CS} @ {SPI_HZ} Hz)")
    failed = 0
    for name, fn in CHECKS:
        try:
            result = fn()
            detail = f" -> {result}" if result is not None else ""
            print(f"  PASS  {name}{detail}")
        except Exception as e:  # noqa: BLE001 - report every check, don't abort early
            print(f"  FAIL  {name}: {e}")
            failed += 1
    print("RESULT:", "PASS" if failed == 0 else f"FAIL ({failed} check(s))")
    return 1 if failed else 0


# --- pytest surface (skips when no hardware configured) ----------------------

try:
    import pytest

    _needs_hw = pytest.mark.skipif(
        not os.environ.get("PROBE_PI_HOST"),
        reason="PROBE_PI_HOST unset — Probe V0 hardware e2e",
    )

    @_needs_hw
    def test_pi_reachable():
        assert check_reachable()

    @_needs_hw
    def test_spidev_present():
        check_spidev_present()

    @_needs_hw
    def test_spi_master_registered():
        check_spi_master()

    @_needs_hw
    def test_spi_transfer_roundtrips():
        rx = check_spi_transfer()
        assert len(rx) == 4
except ImportError:
    pass


if __name__ == "__main__":
    sys.exit(main())
