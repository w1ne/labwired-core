#!/usr/bin/env python3
"""Run exact STM32G0B1RE UART images through the native C/Q oracle.

This does not build firmware, synthesize a result, or substitute a missing ELF.
An input manifest contains ``images`` with id, backend (cmsis-irq/cmsis-dma/
hal-dma), baud, elf, sha256 and layout. Relative paths resolve from the manifest.
Optional compiler/source/HAL provenance is retained as declared metadata.
The native fixture's ideal supply assumption must be explicitly declared as
power_model: ideal_vdd; that assumption is retained in every result.

By default the manifest must cover both DMA backends at COM3 and COM2. --only
and --cycles below 100 are explicitly recorded as diagnostic runs, never a full
matrix pass. Each image is hash-checked before and after execution. The native
test must execute exactly once and report the observed firmware backend and
independent model DMA transfer counts.
The core runs with event-scheduler, as the browser and default CLI do.
"""

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import subprocess
import sys
import time

BACKENDS = {"cmsis-irq": 0, "cmsis-dma": 1, "hal-dma": 2}
REQUIRED = {(backend, baud) for backend in ("cmsis-dma", "hal-dma")
            for baud in (230400, 38400)}
TEST = "real_g0_firmware_timed_cq_session"
CORE_FEATURES = ["event-scheduler"]


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def load_images(path):
    manifest_bytes = path.read_bytes()
    manifest = json.loads(manifest_bytes)
    require(isinstance(manifest, dict) and isinstance(manifest.get("images"), list),
            "manifest must contain an images array")
    seen = set()
    images = []
    for item in manifest["images"]:
        require(isinstance(item, dict), "each image must be an object")
        row = dict(item)
        label = row.get("id", "")
        require(isinstance(label, str) and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]*", label),
                "image id must be a safe filename")
        require(label not in seen, f"duplicate image id: {label}")
        seen.add(label)
        require(row.get("backend") in BACKENDS, f"{label}: unsupported backend")
        require(row.get("baud") in (230400, 38400), f"{label}: expected COM3 or COM2 baud")
        require(row.get("power_model") == "ideal_vdd",
                f"{label}: the native fixture requires an explicit ideal_vdd supply declaration")
        require(isinstance(row.get("sha256"), str)
                and re.fullmatch(r"[0-9a-f]{64}", row["sha256"]), f"{label}: SHA-256 required")
        for field in ("elf", "layout"):
            require(isinstance(row.get(field), str) and row[field], f"{label}: {field} required")
            artifact = (path.parent / row[field]).resolve()
            require(artifact.is_file(), f"{label}: missing {field}: {artifact}")
            row[field] = str(artifact)
        require(sha256(Path(row["elf"])) == row["sha256"], f"{label}: ELF hash mismatch")
        layout_bytes = Path(row["layout"]).read_bytes()
        layout = json.loads(layout_bytes)
        require(layout.get("elf_sha256") == row["sha256"], f"{label}: layout belongs to another ELF")
        row["layout_sha256"] = hashlib.sha256(layout_bytes).hexdigest()
        images.append(row)
    require(images, "no firmware images supplied")
    return images, hashlib.sha256(manifest_bytes).hexdigest()


def source_snapshot(root):
    def git(*args):
        return subprocess.check_output(["git", "-C", str(root), *args])
    untracked = [name.decode() for name in git("ls-files", "--others", "--exclude-standard", "-z").split(b"\0") if name]
    return {
        "local_head": git("rev-parse", "HEAD").decode().strip(),
        "local_tree": git("rev-parse", "HEAD^{tree}").decode().strip(),
        "tracked_diff_sha256": hashlib.sha256(git("diff", "--binary", "HEAD")).hexdigest(),
        "status": git("status", "--porcelain", "--untracked-files=normal").decode().splitlines(),
        "untracked_sha256": {name: sha256(root / name) for name in untracked if (root / name).is_file()},
        "runner_sha256": sha256(Path(__file__)),
    }


def validate_report(report, row, cycles):
    require(report.get("cpu_hz") == 64000000, "wrong modeled CPU frequency")
    require(report.get("baud") == row["baud"], "wrong UART baud")
    require(report.get("profile") == "type2v_pd32", "wrong protocol profile")
    require(Path(report.get("firmware", "")).resolve() == Path(row["elf"]),
            "report belongs to a different ELF path")
    require(report.get("firmware_dll_timing_enforcement", {}).get("enabled") is True,
            "firmware timing checks are not enabled")
    require(report.get("firmware_dll_timing_enforcement", {}).get("evidence")
            == "ELF-resolved rig_timing_enforced RAM value",
            "timing state must come from the executing ELF, not a build declaration")
    for phase in ("firmware_dll_initial", "firmware_dll_final"):
        require(report.get(phase, {}).get("values", {}).get("dll.enforce_timing") == 1,
                f"{phase}: observed DLL timing enforcement missing or disabled")
    counters = report["firmware_dll_final"]["values"]
    for counter in ("timing_errors", "timeout_errors", "t_ren_violations",
                    "t_byte_violations", "t_cycle_violations"):
        require(counters.get("dll." + counter) == 0,
                f"final DLL {counter} missing or nonzero")
    require(report.get("benchmark_read_count") == cycles, "READ cases missing")
    require(report.get("benchmark_write_count") == cycles, "WRITE cases missing")
    expected = 2 * cycles + 8
    require(report.get("cyclic_count") == expected, "cyclic or application-cadence cases missing")
    require(report.get("pd_payload_bytes_checked") == expected * 32,
            "complete current-request PD32 validation missing")
    require(report.get("checksum_validated_reply_count") == expected + 3,
            "startup or reply-checksum validation missing")
    gaps = report.get("fidelity_gaps")
    require(isinstance(gaps, list), "fidelity report missing")
    require(all(isinstance(gap, dict) and gap.get("kind") == "unpowered_rail_assumed"
                and row.get("power_model") == "ideal_vdd" for gap in gaps),
            "unmodeled behavior or an undeclared supply assumption was exercised")
    integrity = report.get("uart_receive_integrity", {})
    require(integrity.get("parity_errors") == 0 and integrity.get("framing_errors") == 0,
            "UART parity/framing error")
    transactions = report.get("transactions", [])
    require(len(transactions) == expected + 3, "transaction records missing")
    reply_bytes = sum(len(t.get("reply", [])) for t in transactions)
    require(integrity.get("received_reply_characters") == reply_bytes,
            "received UART character count differs from observed reply bytes")
    minimum, maximum = 1000000 / row["baud"], 10000000 / row["baud"]
    for transaction in transactions:
        ta = transaction.get("ta_us")
        require(isinstance(ta, (int, float)) and math.isfinite(ta)
                and minimum <= ta <= maximum, f"C/Q tA outside 1..10 Tbit: {ta}")
        gap = transaction.get("byte_gap_max_tbit")
        if len(transaction.get("reply", [])) == 1:
            require(gap is None, "one-byte response must not claim a measured inter-byte gap")
        else:
            require(isinstance(gap, (int, float)) and math.isfinite(gap) and 0 <= gap <= 3,
                    f"device byte gap exceeds 3 Tbit: {gap}")
        require(transaction.get("firmware_dll_snapshot", {}).get("values", {})
                .get("dll.enforce_timing") == 1, "DLL timing enforcement missing in transaction")
    cyclic_ta = [t["ta_us"] for t in transactions if t.get("phase") == "OPERATE"]
    require(len(cyclic_ta) == expected, "OPERATE timing records missing")
    require(report.get("cyclic_ta_min_us") == min(cyclic_ta)
            and report.get("cyclic_ta_max_us") == max(cyclic_ta),
            "aggregate cyclic timing does not match the actual transactions")
    transport = report.get("observed_transport", {})
    require(transport.get("backend") == BACKENDS[row["backend"]],
            "observed ELF backend missing or different from requested backend")
    dma = transport.get("dma_model", {})
    if row["backend"] != "cmsis-irq":
        request_bytes = sum(len(t["request"]) for t in transactions)
        require(dma.get("tx_transfers") == reply_bytes,
                "model DMA TX item count differs from observed reply bytes")
        require(isinstance(dma.get("rx_transfers"), int) and dma["rx_transfers"] >= request_bytes,
                "model DMA RX transfers do not cover the actual master requests")
        driver = transport.get("firmware_stats", {})
        for field in ("tx_started", "tx_dma_completed", "tx_uart_completed"):
            require(driver.get(field) == len(transactions),
                    f"driver {field} differs from actual completed replies")
        for field in ("tx_aborts", "dma_errors", "uart_errors", "rx_overruns"):
            require(driver.get(field) == 0, f"clean-run driver {field} missing or nonzero")
    return {
        "cyclic_replies": expected,
        "ta_min_us": min(cyclic_ta),
        "ta_max_us": max(cyclic_ta),
        "pd_bytes_checked": report["pd_payload_bytes_checked"],
        "uart_reply_characters": integrity["received_reply_characters"],
        "observed_transport": transport,
        "fidelity_gaps": gaps,
        "declared_power_model": row.get("power_model"),
        "limits": report.get("limits", []),
    }


def run_image(row, args, core, output):
    report_path = output / (row["id"] + ".json")
    log_path = output / (row["id"] + ".log")
    # The native test also removes its old auxiliary files; remove its report
    # before invoking cargo so an unexecuted or missing test cannot reuse it.
    report_path.unlink(missing_ok=True)
    env = {k: v for k, v in os.environ.items() if not k.startswith("IOLINKI_G0_")}
    env.update({
        "IOLINKI_G0_ELF": row["elf"], "IOLINKI_G0_HZ": "64000000",
        "IOLINKI_G0_BAUD": str(row["baud"]), "IOLINKI_G0_PROFILE": "type2v",
        "IOLINKI_G0_CYCLES": str(args.cycles), "IOLINKI_G0_DLL_TIMING_ENFORCED": "1",
        "IOLINKI_G0_DLL_LAYOUT": row["layout"], "IOLINKI_G0_REPORT": str(report_path),
        "IOLINKI_G0_EXPECT_BACKEND": str(BACKENDS[row["backend"]]),
    })
    command = [args.cargo, "test", "--offline", "-p", "labwired-core",
               "--features", ",".join(CORE_FEATURES), "--test",
               "g0_customer_timing", "--", TEST, "--exact", "--ignored", "--nocapture"]
    result = {"id": row["id"], "backend": row["backend"], "baud": row["baud"],
              "elf_sha256": row["sha256"], "layout_sha256": row["layout_sha256"],
              "declared_artifact": row,
              "report": str(report_path), "log": str(log_path), "passed": False}
    start = time.monotonic()
    try:
        require(sha256(Path(row["elf"])) == row["sha256"], "ELF changed before execution")
        require(sha256(Path(row["layout"])) == row["layout_sha256"], "layout changed before execution")
        print(f"Running {row['id']} ({row['backend']}, {row['baud']} baud)", flush=True)
        with log_path.open("w") as log:
            completed = subprocess.run(command, cwd=core, env=env, stdout=log,
                                       stderr=subprocess.STDOUT, timeout=args.timeout)
        require(completed.returncode == 0, f"native oracle exited {completed.returncode}; see log")
        require(re.search(r"test result: ok\. 1 passed; 0 failed; 0 ignored;", log_path.read_text()),
                "the exact native test did not execute and pass once")
        require(sha256(Path(row["elf"])) == row["sha256"], "ELF changed during execution")
        require(sha256(Path(row["layout"])) == row["layout_sha256"], "layout changed during execution")
        require(report_path.is_file(), "native test produced no report")
        result.update(validate_report(json.loads(report_path.read_text()), row, args.cycles))
        result["report_sha256"] = sha256(report_path)
        result["passed"] = True
    except (ValueError, OSError, subprocess.TimeoutExpired) as error:
        result["error"] = str(error)
    result["elapsed_seconds"] = round(time.monotonic() - start, 3)
    print(json.dumps({k: result[k] for k in ("id", "passed", "ta_min_us", "ta_max_us", "error")
                      if k in result}), flush=True)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--core-root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--cargo", default="cargo")
    parser.add_argument("--cycles", type=int, default=100, help="READ and WRITE cases per image")
    parser.add_argument("--timeout", type=int, default=1800, help="maximum seconds per native run")
    parser.add_argument("--only", action="append", default=[], help="select id; explicitly diagnostic")
    parser.add_argument("--keep-going", action="store_true", help="run remaining images after failure")
    args = parser.parse_args()
    require(args.cycles > 0 and args.timeout > 0, "cycles and timeout must be positive")
    manifest_path = args.manifest.resolve()
    images, manifest_hash = load_images(manifest_path)
    if args.only:
        require(set(args.only) <= {row["id"] for row in images}, "unknown --only image id")
        images = [row for row in images if row["id"] in args.only]
    covered = {(row["backend"], row["baud"]) for row in images}
    if not args.only:
        require(REQUIRED <= covered, "manifest must cover cmsis-dma and hal-dma at COM3 and COM2")
    core, output = args.core_root.resolve(), args.output.resolve()
    require(output != core and core not in output.parents,
            "output must be outside the core checkout so reports do not change source identity")
    output.mkdir(parents=True, exist_ok=True)
    source_before = source_snapshot(core)
    require(sha256(manifest_path) == manifest_hash, "manifest changed after parsing")
    summary = {"schema_version": 1, "manifest_sha256": manifest_hash,
               "core_features": CORE_FEATURES,
               "core_before": source_before, "cycles_per_direction": args.cycles,
               "full_matrix_pass": False, "results": [],
               "scope": "simulation; no IAR, physical timing or Beckhoff acceptance claim"}
    for row in images:
        summary["results"].append(run_image(row, args, core, output))
        (output / "matrix-summary.json").write_text(json.dumps(summary, indent=2) + "\n")
        if not summary["results"][-1]["passed"] and not args.keep_going:
            break
    summary["core_after"] = source_snapshot(core)
    summary["source_changed_during_run"] = summary["core_after"] != source_before
    summary["manifest_changed_during_run"] = sha256(manifest_path) != manifest_hash
    summary["not_run"] = [row["id"] for row in images[len(summary["results"]):]]
    selected_pass = (len(summary["results"]) == len(images)
                     and all(row["passed"] for row in summary["results"])
                     and not summary["source_changed_during_run"]
                     and not summary["manifest_changed_during_run"])
    summary["all_selected_passed"] = selected_pass
    summary["full_matrix_pass"] = bool(selected_pass and REQUIRED <= covered
                                       and args.cycles >= 100 and not args.only)
    (output / "matrix-summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps({"all_selected_passed": selected_pass,
                      "full_matrix_pass": summary["full_matrix_pass"],
                      "summary": str(output / "matrix-summary.json")}), flush=True)
    return 0 if selected_pass else 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        print(f"matrix preflight failed: {error}", file=sys.stderr)
        sys.exit(2)
