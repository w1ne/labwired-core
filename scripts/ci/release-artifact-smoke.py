#!/usr/bin/env python3
"""Run a packaged CLI and a framed DAP session on their native host."""

from __future__ import annotations

import argparse
import json
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path


def dap_frame(message: dict) -> bytes:
    payload = json.dumps(message).encode("utf-8")
    return f"Content-Length: {len(payload)}\r\n\r\n".encode("ascii") + payload


def dap_messages(data: bytes) -> list[dict]:
    messages = []
    while data:
        header, separator, remainder = data.partition(b"\r\n\r\n")
        if not separator:
            raise RuntimeError("DAP output has an incomplete header")
        lengths = [
            line.partition(b":")[2].strip()
            for line in header.split(b"\r\n")
            if line.partition(b":")[0].lower() == b"content-length"
        ]
        if len(lengths) != 1:
            raise RuntimeError("DAP output needs exactly one Content-Length header")
        length = int(lengths[0])
        if length <= 0 or len(remainder) < length:
            raise RuntimeError("DAP output has an invalid or incomplete payload")
        message = json.loads(remainder[:length])
        if not isinstance(message, dict):
            raise RuntimeError("DAP payload must be a JSON object")
        messages.append(message)
        data = remainder[length:]
    return messages


def run_binary(command: list[str], directory: Path, data: bytes | None = None) -> bytes:
    # subprocess.run drains both pipes and enforces its timeout on Windows too.
    result = subprocess.run(
        command, cwd=directory, input=data, capture_output=True, timeout=30
    )
    if result.returncode != 0:
        raise RuntimeError(
            f"{command[0]} exited {result.returncode}: "
            f"{result.stderr.decode('utf-8', errors='replace')}"
        )
    return result.stdout


def smoke_archive(archive: Path, version: str, target: str, container: str | None = None) -> None:
    machine = platform.machine().lower()
    machine = {"amd64": "x86_64", "arm64": "aarch64"}.get(machine, machine)
    system = {"win32": "windows"}.get(sys.platform, sys.platform)
    host = f"{system}-{machine}"
    if host != target:
        raise RuntimeError(f"Archive target {target} must run on a native host, got {host}")
    if container and system != "linux":
        raise RuntimeError("Container smoke requires a native Linux host")

    suffix = ".exe" if system == "windows" else ""
    expected = {f"labwired{suffix}", f"labwired-dap{suffix}"}
    with tempfile.TemporaryDirectory(prefix="labwired-release-smoke-") as temporary:
        directory = Path(temporary)
        with tarfile.open(archive, "r:gz") as bundle:
            members = bundle.getmembers()
            if len(members) != 2 or {member.name for member in members} != expected:
                raise RuntimeError(f"Archive must contain exactly {sorted(expected)}")
            for member in members:
                if not member.isfile() or not member.mode & 0o111:
                    raise RuntimeError(f"Archive entry is not executable: {member.name}")
                source = bundle.extractfile(member)
                if source is None:
                    raise RuntimeError(f"Cannot read archive entry: {member.name}")
                destination = directory / member.name
                with source:
                    destination.write_bytes(source.read())
                destination.chmod(member.mode & 0o777)

        runtime_directory = Path("/d") if container else directory
        prefix = (
            [
                "docker", "run", "--rm", "-i", "--mount",
                f"type=bind,source={directory},target=/d,readonly", container,
            ]
            if container else []
        )
        cli = [*prefix, str(runtime_directory / f"labwired{suffix}")]
        reported = run_binary([*cli, "--version"], directory).decode().strip()
        if reported != f"labwired-cli {version}":
            raise RuntimeError(f"Expected CLI version {version}, got {reported!r}")

        # Synthetic CI chips are deliberately excluded from the CLI catalog.
        # Copy the committed descriptor with the ELF; no source tree is mounted.
        root = Path(__file__).resolve().parents[2]
        shutil.copyfile(root / "tests/fixtures/uart-ok-thumbv7m.elf", directory / "firmware.elf")
        shutil.copyfile(
            root / "configs/chips/ci-fixture-cortex-m3-uart1.yaml", directory / "chip.yaml"
        )
        (directory / "system.yaml").write_text(json.dumps({
            "name": "release-smoke", "chip": "./chip.yaml", "external_devices": [],
        }))
        (directory / "smoke.yaml").write_text(json.dumps({
            "schema_version": "1.0",
            "inputs": {"firmware": "firmware.elf", "system": "system.yaml"},
            "limits": {"max_steps": 1000},
            "assertions": [{"uart_contains": "OK"}],
        }))
        output = Path("/tmp/labwired-release-smoke") if container else directory / "output"
        run_binary([
            *cli, "test", "--script", str(runtime_directory / "smoke.yaml"),
            "--output-dir", str(output), "--no-key", "--no-uart-stdout",
        ], directory)

        requests = [
            {
                "seq": 1,
                "type": "request",
                "command": "initialize",
                "arguments": {"adapterID": "labwired", "clientID": "release-smoke"},
            },
            {
                "seq": 2,
                "type": "request",
                "command": "disconnect",
                "arguments": {"terminateDebuggee": False},
            },
        ]
        dap = [*prefix, str(runtime_directory / f"labwired-dap{suffix}")]
        output = run_binary(
            dap, directory, b"".join(dap_frame(request) for request in requests)
        )
        messages = dap_messages(output)
        responses = [message for message in messages if message.get("type") == "response"]
        if len(responses) != len(requests):
            raise RuntimeError(f"Expected two DAP responses, got {responses!r}")
        for request in requests:
            matches = [
                response
                for response in responses
                if response.get("request_seq") == request["seq"]
                and response.get("command") == request["command"]
                and response.get("success") is True
            ]
            if len(matches) != 1:
                raise RuntimeError(f"DAP {request['command']} did not succeed: {responses!r}")
        if not any(
            message.get("type") == "event" and message.get("event") == "initialized"
            for message in messages
        ):
            raise RuntimeError("DAP did not emit its initialized event")

    print(f"{target} ({container or 'host'}): {reported}; firmware UART OK; DAP initialize/disconnect passed")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", required=True, type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--platform", required=True)
    parser.add_argument("--container", help="Use this pre-pulled Linux image for execution")
    args = parser.parse_args()
    smoke_archive(args.archive, args.version, args.platform, args.container)


if __name__ == "__main__":
    main()
