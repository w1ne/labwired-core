#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""C9 diagnostic: retain the unchanged AVR low/high workload's attribution.

Run only in hosted CI. This producer is separate from board_perf.py: it never
updates a baseline, changes a source profile, or writes an issue. Readers of
its originals must not import this module or execute the simulator.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import statistics
import subprocess
import sys
import time

SNAPSHOTS = {
    "before-sync": "f1ca219d08080f4d93a73a3a4c5630e0824db799",
    "sync-merge": "6e03dd12e22ceb534ec1bc494c360b72209ca94f",
    "current": "715e1272d639aff3d12c0c73ff5f00e9a94d3371",
    "upstream-source": "49f656f5233d109a8d357e6c5e0cec9e788decd5",
}
LOW, HIGH = 200_000, 1_200_000
INPUTS = ("crates/firmware-perf-spin-avr/main.c", "configs/chips/atmega328p.yaml")
BUILD = [
    "cargo",
    "build",
    "--locked",
    "--release",
    "-p",
    "labwired-cli",
    "--features",
    "event-scheduler",
    "--verbose",
]
ANNOTATE = [
    "callgrind_annotate",
    "--auto=no",
    "--inclusive=no",
    "--show=Ir",
    "--show-percs=no",
    "--threshold=100",
]
BATCH = re.compile(
    r"^\[batched\] instructions=(\d+) batches=(\d+) "
    r"steps_per_batch=([\d.]+) tick_interval=(\d+)",
    re.M,
)


def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def parse_run(stderr: str, profile: str, mode: str, steps: int) -> dict:
    refs = re.findall(r"I\s+refs:\s+([\d,]+)", stderr)
    summaries = re.findall(r"^summary:\s+(\d+)\s*$", profile, re.M)
    if len(refs) != 1 or len(summaries) != 1:
        raise ValueError("Exactly one original I-refs and profile summary required")
    instructions = int(refs[0].replace(",", ""))
    if instructions <= 0 or instructions != int(summaries[0]):
        raise ValueError("Original stderr/profile instruction totals disagree")
    if re.findall(r"^events:\s*(.+)$", profile, re.M) != ["Ir"]:
        raise ValueError("Expected the unchanged Ir-only profiling event")
    terminal = re.findall(
        r"labwired-cli run \(avr(, batched)?\): "
        r"reached --max-steps (\d+); pc=0x([0-9a-f]+)",
        stderr,
    )
    if len(terminal) != 1 or int(terminal[0][1]) != steps:
        raise ValueError("Missing exact finite AVR terminal witness")
    matches = list(BATCH.finditer(stderr))
    result = {
        "irefs": instructions,
        "requestedSteps": steps,
        "terminalPc": terminal[0][2],
    }
    if mode == "step":
        if terminal[0][0] or matches:
            raise ValueError("Step invocation entered the batch path")
    elif mode == "batch":
        if not terminal[0][0] or len(matches) != 1:
            raise ValueError("Batch path must have its original summary")
        proof = matches[0]
        if int(proof[1]) != steps or not 0 < int(proof[2]) <= steps:
            raise ValueError("Batch retirement/count differs from requested work")
        width = float(proof[3])
        if width < 1 or int(proof[4]) < 1:
            raise ValueError(
                "Batch execution must retain its real width and tick interval"
            )
        if abs(steps / int(proof[2]) - width) > 0.005001:
            raise ValueError("Printed batch width differs from the original counts")
        result.update(
            instructions=int(proof[1]),
            batches=int(proof[2]),
            stepsPerBatch=float(proof[3]),
            tickInterval=int(proof[4]),
        )
    else:
        raise ValueError("Unknown execution mode")
    return result


def profile_command(
    cli: Path, chip: Path, firmware: Path, output: Path, mode: str, steps: int
) -> list[str]:
    if mode not in ("step", "batch") or steps not in (LOW, HIGH):
        raise ValueError("Only the declared original modes/bounds are admitted")
    return [
        "valgrind",
        "--tool=callgrind",
        f"--callgrind-out-file={output}",
        "--cache-sim=no",
        "--branch-sim=no",
        "--collect-jumps=yes",
        "--dump-instr=yes",
        str(cli),
        "run",
        "--chip",
        str(chip),
        "--firmware",
        str(firmware),
        "--max-steps",
        str(steps),
        *(["--batched"] if mode == "batch" else []),
    ]


class Commands:
    def __init__(self, output: Path):
        self.output = output
        self.sequence = 0

    def run(
        self,
        name: str,
        argv: list[str],
        cwd: Path,
        *,
        env: dict | None = None,
        timeout: int = 120,
    ) -> bytes:
        self.sequence += 1
        stem = f"{self.sequence:02d}-{name}"
        row = {
            "index": self.sequence,
            "name": name,
            "argv": argv,
            "cwd": str(cwd),
            "timeoutSeconds": timeout,
            "modeEnvironment": {
                k: (env or os.environ).get(k) for k in ["LABWIRED_ARM_SINGLE_STEP"]
            },
        }
        with (self.output / "commands.jsonl").open("a") as stream:
            stream.write(json.dumps({**row, "phase": "before"}) + "\n")
        print(json.dumps({"command": row}), flush=True)
        started = time.monotonic()
        # Keep output files even on a build, guest or profiler failure.
        with (
            (self.output / (stem + ".stdout")).open("xb") as stdout,
            (self.output / (stem + ".stderr")).open("xb") as stderr,
        ):
            completed = subprocess.run(
                argv, cwd=cwd, env=env, stdout=stdout, stderr=stderr, timeout=timeout
            )
        with (self.output / "commands.jsonl").open("a") as stream:
            stream.write(
                json.dumps(
                    {
                        **row,
                        "phase": "after",
                        "returncode": completed.returncode,
                        "elapsedSeconds": time.monotonic() - started,
                    }
                )
                + "\n"
            )
        if completed.returncode:
            raise RuntimeError(
                f"{name} exited {completed.returncode}; originals retained"
            )
        return (self.output / (stem + ".stdout")).read_bytes()


def produce(source: Path, output: Path, variant: str) -> dict:
    expected = SNAPSHOTS[variant]
    for key in os.environ:
        if key in {
            "RUSTFLAGS",
            "CARGO_ENCODED_RUSTFLAGS",
            "RUSTC_WRAPPER",
        } or key.startswith("CARGO_PROFILE_"):
            if os.environ[key]:
                raise ValueError(f"Undeclared compiler override: {key}")
    commands = Commands(output)
    identity = (
        commands.run("source", ["git", "rev-parse", "HEAD"], source).decode().strip()
    )
    if identity != expected:
        raise ValueError("Selected source differs from the frozen snapshot")
    if commands.run("source-status", ["git", "status", "--porcelain"], source):
        raise ValueError("Source snapshot is dirty before building")
    controller = Path(__file__).resolve().parents[2]
    controller_source = (
        commands.run("controller-source", ["git", "rev-parse", "HEAD"], controller)
        .decode()
        .strip()
    )
    for relative in INPUTS:
        if sha(source / relative) != sha(controller / relative):
            raise ValueError(
                f"The common authored measurement input changed: {relative}"
            )
    for relative in [*INPUTS, "Cargo.toml", "Cargo.lock", ".cargo/config.toml"]:
        path = source / relative
        if path.exists():
            target = output / "source" / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(path.read_bytes())
    versions = {}
    for name, argv in [
        ("rustc", ["rustc", "-vV"]),
        ("cargo", ["cargo", "-Vv"]),
        ("valgrind", ["valgrind", "--version"]),
        ("avr-gcc", ["avr-gcc", "--version"]),
        ("objdump", ["objdump", "--version"]),
    ]:
        versions[name] = commands.run(name, argv, source).decode()
    if not versions["rustc"].startswith("rustc 1.95.0 "):
        raise ValueError("The declared Rust 1.95.0 compiler is required")
    commands.run("build", BUILD, source, timeout=1500)
    cli = source / "target/release/labwired"
    cli_sha = sha(cli)
    commands.run("symbols", ["nm", "--defined-only", "-n", "-C", str(cli)], source)
    commands.run(
        "disassembly",
        ["objdump", "-d", "-C", "--no-show-raw-insn", str(cli)],
        source,
        timeout=180,
    )
    firmware = output / "firmware.elf"
    commands.run(
        "firmware",
        [
            "avr-gcc",
            "-mmcu=atmega328p",
            "-Os",
            "-nostdlib",
            "-Wl,--section-start=.text=0",
            "-Wl,-e,main",
            "-o",
            str(firmware),
            str(source / INPUTS[0]),
        ],
        source,
    )
    commands.run("firmware-disassembly", ["avr-objdump", "-d", str(firmware)], source)
    samples = []
    for mode, repeats in [("step", 1), ("batch", 3)]:
        for repeat in range(repeats):
            pair = []
            for steps in [LOW, HIGH]:
                name = f"{mode}-{repeat}-{steps}"
                profile = output / (name + ".callgrind")
                env = dict(os.environ)
                env.pop("LABWIRED_ARM_SINGLE_STEP", None)
                if mode == "step":
                    env["LABWIRED_ARM_SINGLE_STEP"] = "1"
                argv = profile_command(
                    cli, source / INPUTS[1], firmware, profile, mode, steps
                )
                commands.run(name, argv, source, env=env, timeout=180)
                stderr = (output / f"{commands.sequence:02d}-{name}.stderr").read_text()
                parsed = parse_run(stderr, profile.read_text(), mode, steps)
                commands.run(name + "-annotation", [*ANNOTATE, str(profile)], source)
                pair.append(parsed)
            slope = (pair[1]["irefs"] - pair[0]["irefs"]) / (HIGH - LOW)
            if slope <= 0:
                raise ValueError("Nonpositive instruction slope")
            samples.append(
                {
                    "mode": mode,
                    "repeat": repeat,
                    "low": pair[0],
                    "high": pair[1],
                    "irPerStep": slope,
                }
            )
    if sha(cli) != cli_sha:
        raise ValueError("The measured executable changed during profiling")
    return {
        "schema": "labwired.avr-layout-profile.v1",
        "variant": variant,
        "source": identity,
        "cliSha256": cli_sha,
        "firmwareSha256": sha(firmware),
        "controllerSource": controller_source,
        "producerSha256": sha(Path(__file__)),
        "versions": versions,
        "bounds": [LOW, HIGH],
        "samples": samples,
        "medianIrPerStep": {
            mode: statistics.median(
                row["irPerStep"] for row in samples if row["mode"] == mode
            )
            for mode in ["step", "batch"]
        },
        "runner": {
            key: os.environ.get(key)
            for key in ["ImageOS", "ImageVersion", "RUNNER_OS", "RUNNER_ARCH"]
        },
        "scope": "Native authored spin diagnostic, not full baseline, RTx or physical timing qualification",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--variant", choices=SNAPSHOTS, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    status = 0
    try:
        result = produce(args.source.resolve(), output, args.variant)
    except Exception as error:
        result = {
            "schema": "labwired.avr-layout-profile.v1",
            "variant": args.variant,
            "failed": True,
            "errorType": type(error).__name__,
            "error": str(error),
        }
        status = 1
    (output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    files = {
        str(path.relative_to(output)): {
            "sha256": sha(path),
            "bytes": path.stat().st_size,
        }
        for path in sorted(output.rglob("*"))
        if path.is_file()
    }
    (output / "inventory.json").write_text(json.dumps(files, indent=2) + "\n")
    print(json.dumps(result), flush=True)
    return status


if __name__ == "__main__":
    sys.exit(main())
