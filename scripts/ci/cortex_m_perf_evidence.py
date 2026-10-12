"""Optional, bounded three-board A/B evidence around the unchanged perf tool.

No baseline updates, global performance verdict or physical capture claims.
"""

import argparse
from contextlib import nullcontext
import hashlib
import importlib.util
import itertools
import json
import math
import os
from pathlib import Path
import re
import selectors
import signal
import statistics
import subprocess
import sys
import time

BOARDS = ("nrf52840", "stm32f405", "stm32l476")
COUNTS = (200000, 1200000)
SIDES = ("base", "head")
MODES = ("step", "batch")


def validate_sha(value):
    if re.fullmatch(r"[0-9a-f]{40}", value) is None:
        raise ValueError("performance qualification requires an exact lowercase 40-hex SHA")


class Budget:
    def __init__(self, limit=8 * 1024 * 1024):
        self.remaining = limit


def collect(command, *, cwd=None, env=None, timeout=300, stream_limit=262144, budget=None):
    """Retain both pipes incrementally; terminate only this owned session."""
    if not command or timeout <= 0 or stream_limit <= 0:
        raise ValueError("invalid bounded command")
    budget = budget if budget is not None else Budget()
    buffers = {"stdout": bytearray(), "stderr": bytearray()}
    # Linux WNOWAIT keeps the directly owned leader unreaped, reserving its PID
    # and session/group identity until every signal is finished. Popen.poll/wait
    # must not run earlier. Reject unsupported/auto-reaping hosts before birth;
    # no portable fallback can safely signal a nominal, possibly reused PGID.
    if (sys.platform != "linux" or not callable(getattr(os, "waitid", None))
            or not all(hasattr(os, name) for name in ("P_PID", "WEXITED", "WNOHANG", "WNOWAIT", "CLD_EXITED", "CLD_KILLED", "CLD_DUMPED"))
            or not all(callable(getattr(os, name, None)) for name in ("getpgid", "getsid"))
            or signal.getsignal(signal.SIGCHLD) != signal.SIG_DFL):
        raise ValueError("bounded collection requires Linux unreaped-child ownership")
    process = selector = None
    original_error = None
    cleanup_errors = []
    ownership_lost = False
    observed_exit = None

    def exited():
        nonlocal ownership_lost, observed_exit
        if ownership_lost:
            raise RuntimeError("owned producer lifetime was lost")
        try:
            state = os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
        except ChildProcessError:
            ownership_lost = True
            raise
        if state is not None and state.si_pid != process.pid:
            ownership_lost = True
            raise RuntimeError("owned producer identity changed")
        if state is not None:
            if state.si_code == os.CLD_EXITED:
                observed_exit = state.si_status
            elif state.si_code in (os.CLD_KILLED, os.CLD_DUMPED):
                observed_exit = -state.si_status
            else:
                raise RuntimeError("invalid owned producer terminal state")
        return state is not None

    def reap():
        # Never attach a direct wait to a PID after ECHILD/identity loss.
        if not ownership_lost:
            exited()
            process.wait(timeout=0.2)

    def signal_owned(sig):
        # waitid also rejects ECHILD/auto-reap; no signal follows lost ownership.
        exited()
        if os.getpgid(process.pid) != process.pid or os.getsid(process.pid) != process.pid:
            raise RuntimeError("owned producer session/group changed")
        try:
            os.killpg(process.pid, sig)
        except ProcessLookupError:
            # An unreaped, verified leader can have no remaining signalable peers.
            pass

    def attempt(cleanup):
        try:
            cleanup()
        except BaseException as error:
            cleanup_errors.append(error)
    forced = None
    try:
        selector = selectors.DefaultSelector()
        process = subprocess.Popen(command, cwd=cwd, env=env, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, start_new_session=True)
        selector.register(process.stdout, selectors.EVENT_READ, "stdout")
        selector.register(process.stderr, selectors.EVENT_READ, "stderr")
        deadline = time.monotonic() + timeout
        while selector.get_map() or not exited():
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                forced = 124
                break
            for key, _ in selector.select(min(remaining, 0.1)):
                chunk = os.read(key.fd, 16384)
                if not chunk:
                    selector.unregister(key.fileobj)
                    continue
                buffer = buffers[key.data]
                available = min(stream_limit - len(buffer), budget.remaining)
                retained = min(len(chunk), available)
                buffer.extend(chunk[:retained])
                budget.remaining -= retained
                if retained < len(chunk):
                    forced = 125
                    break
            if forced is not None:
                break
    except BaseException as error:
        original_error = error
        raise
    finally:
        if process is not None:
            attempt(lambda: signal_owned(signal.SIGTERM))
            # The existing TERM grace is bounded, but does not reap the leader.
            def grace():
                if not exited():
                    time.sleep(0.2)
            attempt(grace)
            attempt(lambda: signal_owned(signal.SIGKILL))
            # Only after all group signals: bounded direct-child reap.
            attempt(reap)
        if selector is not None:
            attempt(selector.close)
        if process is not None:
            attempt(process.stdout.close)
            attempt(process.stderr.close)
        # WNOWAIT terminal status remains original evidence even if reap fails.
        producer_exit = process.returncode if process is not None and process.returncode is not None else observed_exit
        if cleanup_errors:
            if original_error is not None:
                original_error.add_note("Owned cleanup also failed: " + ", ".join(type(e).__name__ for e in cleanup_errors))
            elif forced is None and producer_exit in (None, 0):
                raise cleanup_errors[0]
            # A pre-existing producer/timeout/overflow failure keeps its exact
            # status. Cleanup can never turn that failure into a success.
    result = subprocess.CompletedProcess(command, forced if forced is not None else producer_exit,
                                         bytes(buffers["stdout"]), bytes(buffers["stderr"]))
    result.producer_exit_code = producer_exit
    return result


def small_json(path, limit=262144):
    path = Path(path)
    if path.stat().st_size > limit:
        raise ValueError(f"oversized JSON evidence: {path}")
    return json.loads(path.read_text())


def sha256(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(65536), b""):
            h.update(block)
    return h.hexdigest()


def normalize_stderr(data):
    text = re.sub(rb"\x1b\[[0-9;]*m", b"", data)
    text = re.sub(rb"(?m)^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:.]+Z ", b"", text)
    return b"\n".join(line for line in text.splitlines() if not line.startswith(b"[batched] instructions="))


def bounded_argv(command):
    if not isinstance(command, (list, tuple)) or not 1 <= len(command) <= 24 or any(not isinstance(arg, str) or "\0" in arg or len(arg.encode()) > 2048 for arg in command) or sum(len(arg.encode()) for arg in command) > 8192:
        raise ValueError("oversized/invalid performance invocation arguments")
    return list(command)


def validate_identity(folder, steps, base_sha, head_sha, suite="cortex-m"):
    counts = steps.split()
    if suite not in ("cortex-m", "all") or counts != ["5000000", "40000000"]:
        raise ValueError("qualification requires the complete reviewed Cortex-M identity counts")
    cells = [f"cortex-m-{b}-{n}-{m}" for b, n, m in itertools.product(BOARDS, counts, ("step", "batched"))]
    positive = stable = unstable = 0
    for side, repeats, source in (("base", (1, 2), base_sha), ("head", (1,), head_sha)):
        path = Path(folder) / f"identity-{side}"
        expected = {"sha"} | {f"{cell}-p{r}.{s}" for cell, r, s in itertools.product(cells, repeats, ("exit", "stdout", "stderr", "bus.json"))}
        if suite == "all":
            expected |= {f"esp32s3-romboot-{n}-{m}-p{r}.{s}" for n, m, r, s in itertools.product(counts, ("step", "batched"), repeats, ("exit", "stdout", "stderr"))}
        if not path.is_dir() or {p.name for p in path.iterdir() if p.is_file()} != expected:
            raise ValueError(f"incomplete or unexpected identity inventory: {side}")
        if (path / "sha").read_text().strip() != source:
            raise ValueError(f"identity source mismatch: {side}")
        for cell, repeat in itertools.product(cells, repeats):
            prefix = path / f"{cell}-p{repeat}"
            if Path(str(prefix) + ".exit").read_text().strip() != "0" or b"TIER1 " not in Path(str(prefix) + ".stdout").read_bytes():
                raise ValueError("identity did not execute positive TIER1 firmware")
            events = small_json(str(prefix) + ".bus.json", 2 * 1024 * 1024)
            if not isinstance(events, list) or not events:
                raise ValueError("empty/non-list JSON bus trace")
            uart = bytearray()
            positive_uart = bytearray()
            for event in events:
                if not isinstance(event, dict) or type(event.get("cycle")) is not int or event["cycle"] < 0:
                    raise ValueError("invalid negative/noninteger bus cycle")
                payload = event.get("payload", {})
                if event.get("bus", "").startswith("uart") and payload.get("protocol") == "uart" and payload.get("direction") == "tx":
                    byte = payload.get("byte")
                    if type(byte) is not int or not 0 <= byte <= 255:
                        raise ValueError("invalid UART byte")
                    uart.append(byte)
                    if event["cycle"] > 0:
                        positive_uart.append(byte)
            if b"TIER1 " not in uart or b"TIER1 " not in positive_uart:
                raise ValueError("no positive-cycle TIER1 UART bus traffic")
            positive += 1
    for cell, stream in itertools.product(cells, ("exit", "stdout", "stderr", "bus.json")):
        values = [(Path(folder) / f"identity-{side}" / f"{cell}-p{repeat}.{stream}").read_bytes() for side, repeat in (("base", 1), ("base", 2), ("head", 1))]
        if stream == "stderr":
            values = [normalize_stderr(v) for v in values]
        if values[0] != values[1]:
            if stream != "stderr":
                raise ValueError("unstable required firmware-visible identity stream")
            unstable += 1
        elif values[0] != values[2]:
            raise ValueError("identity output changed")
        else:
            stable += 1
    return dict(positive_execution_cells=positive, stable_identical_streams=stable, unstable_not_pass_streams=unstable)


def validate_records(rows, min_tick):
    expected = {(side, board, mode, count, repeat) for side, board, mode, count in itertools.product(SIDES, BOARDS, MODES, COUNTS) for repeat in range(3 if mode == "batch" else 1)}
    keys = [(r["side"], r["board"], r["mode"], r["steps"], r["repeat"]) for r in rows]
    if len(keys) != 48 or len(set(keys)) != 48 or set(keys) != expected:
        raise ValueError("incomplete/duplicate/unexpected 48-invocation inventory")
    measurements = []
    for row in rows:
        if row["exit_code"] != 0 or row["producer_exit_code"] != 0 or type(row["ir"]) is not int or row["ir"] <= 0:
            raise ValueError("failed child or invalid Callgrind instruction count")
        argv = bounded_argv(row["argv"])
        if argv[0] != "valgrind" or not Path(row["cwd"]).is_absolute() or str(Path(row["cwd"]).resolve()) != row["cwd"]:
            raise ValueError("missing resolved invocation working directory")
        for flag in ("--chip", "--firmware", "--max-steps", "run"):
            if argv.count(flag) != 1 or argv.index(flag) + 1 >= len(argv):
                raise ValueError("invalid raw invocation flags")
        if Path(argv[argv.index("--chip") + 1]).stem != row["board"] or argv[argv.index("--max-steps") + 1] != str(row["steps"]):
            raise ValueError("raw invocation disagrees with board/work inventory")
        batch = row["mode"] == "batch"
        expected_env = {"present": not batch, "value": None if batch else "1"}
        if row["mode_environment"] != {"LABWIRED_ARM_SINGLE_STEP": expected_env} or row["selected_flags"] != (["--batched"] if batch else []) or argv.count("--batched") != int(batch):
            raise ValueError("raw mode environment/flags disagree with execution mode")
        for field in ("fixture_sha256", "cli_sha256"):
            if re.fullmatch("[0-9a-f]{64}", row[field]) is None:
                raise ValueError("missing immutable binary evidence")
        if row["mode"] == "batch" and (row["instructions"] != row["steps"] or row["batches"] <= 0 or not math.isfinite(row["steps_per_batch"]) or row["steps_per_batch"] <= 0 or row["tick_interval"] < min_tick):
            raise ValueError("invalid retired-work or batch/tick proof")
        if row["mode"] == "step" and row["instructions"] is not None:
            raise ValueError("step unexpectedly ran the batch loop")
    for board in BOARDS:
        if len({r["fixture_sha256"] for r in rows if r["board"] == board}) != 1:
            raise ValueError("fixture bytes changed between observations")
    for side in SIDES:
        if len({r["cli_sha256"] for r in rows if r["side"] == side}) != 1:
            raise ValueError("CLI bytes changed between observations")
        for board, mode in itertools.product(BOARDS, MODES):
            pairs = []
            for repeat in range(3 if mode == "batch" else 1):
                pair = [next(r for r in rows if (r["side"], r["board"], r["mode"], r["steps"], r["repeat"]) == (side, board, mode, count, repeat)) for count in COUNTS]
                if pair[1]["ir"] <= pair[0]["ir"]:
                    raise ValueError("nonpositive instruction-count slope")
                pairs.append((pair[1]["ir"] - pair[0]["ir"]) / (COUNTS[1] - COUNTS[0]))
            if mode == "batch":
                for count in COUNTS:
                    if len({r["ir"] for r in rows if (r["side"], r["board"], r["mode"], r["steps"]) == (side, board, mode, count)}) != 1:
                        raise ValueError("UNSTABLE repeated batch instruction counts")
            measurements.append(dict(side=side, board=board, mode=mode, ir_per_step=statistics.median(pairs)))
    return dict(invocations=48, measurements=measurements, batch_repeated_stability_proven=True, step_repeated_stability_proven=False)


def validate_status(status, side, measurements, code):
    if code not in (0, 1) or any(status.get(k) for k in ("skipped", "unmeasurable", "contract_failures", "regressions")):
        raise ValueError("failed or skipped committed-baseline/mode qualification")
    measured = status.get("measured", {})
    if set(measured) != set(BOARDS) or any(set(modes) != set(MODES) for modes in measured.values()):
        raise ValueError("incomplete measured board/mode status")
    for row in measurements:
        if row["side"] == side and measured[row["board"]][row["mode"]] != round(row["ir_per_step"], 1):
            raise ValueError("status does not match raw counts")
    stale = bool(status.get("stale"))
    if code != int(stale) or status.get("ok") is not (not stale):
        raise ValueError("tool exit/status baseline verdict was lost")
    return dict(tool_exit_code=code, committed_baseline_pass=not stale, stale=status.get("stale", []))


class BoundedRunner:
    def __init__(self, module, side, output, rows, budget, common=None, raw_profiles=None):
        self.module, self.side, self.output, self.rows, self.budget = module, side, Path(output), rows, budget
        self.ordinal = 0
        self.common = common
        self.raw_profiles = raw_profiles

    def run(self, command, **kwargs):
        if set(kwargs) - {"cwd", "env", "capture_output", "text"} or not kwargs.get("capture_output"):
            raise ValueError("unexpected subprocess contract in unchanged perf tool")
        valgrind = command[0] == "valgrind"
        argv = bounded_argv(command) if valgrind else None
        cwd = str(Path(kwargs.get("cwd") or os.getcwd()).resolve())
        environment = kwargs.get("env") if kwargs.get("env") is not None else os.environ
        fixture_before = fixture_after = fixture_error = None
        metadata_capacity = 8192
        if self.common is not None:
            # The collector may consume every remaining byte. Reserve the
            # original exit and fixture proof before starting that producer.
            from cortex_m_perf_diagnostics import reserve
            reserve(self.budget, metadata_capacity)
        if valgrind and self.common is not None:
            fixture = Path(command[command.index("--firmware") + 1])
            chip = Path(command[command.index("--chip") + 1]).stem
            if chip not in BOARDS or fixture.resolve() != self.common.paths["nrf" if chip == "nrf52840" else "stm32"]:
                raise ValueError("board did not receive its exact common memory-map fixture")
            fixture_before = self.common.verify(fixture)
        if valgrind and self.raw_profiles is not None:
            self.raw_profiles.preflight(command,environment,self.module,sys._getframe(1))
        try:
            result = collect(command, cwd=kwargs.get("cwd"), env=kwargs.get("env"), timeout=300 if valgrind else 900, budget=self.budget)
        finally:
            if fixture_before is not None:
                try:
                    fixture_after = self.common.verify(fixture)
                except (ValueError,OSError) as error:
                    fixture_error = error
        self.ordinal += 1
        prefix = self.output / f"{self.side}-child-{self.ordinal:02}"
        prefix.with_suffix(".stdout").write_bytes(result.stdout)
        prefix.with_suffix(".stderr").write_bytes(result.stderr)
        child_proof = dict(exit_code=result.returncode, producer_exit_code=result.producer_exit_code, stdout_bytes=len(result.stdout), stderr_bytes=len(result.stderr))
        if fixture_before is not None:
            def file_identity(proof):
                return {key:proof[key] for key in ("path","bytes","sha256")} if proof is not None else None
            child_proof.update(fixture_before=file_identity(fixture_before), fixture_after=file_identity(fixture_after))
            if fixture_error is not None:
                child_proof['fixture_validation_error']=str(fixture_error)[:1024]
        data=json.dumps(child_proof)+"\n"
        if self.common is not None and len(data.encode()) > metadata_capacity:
            raise ValueError("oversized reserved diagnostic child metadata")
        prefix.with_suffix(".json").write_text(data)
        if result.returncode != 0:
            raise ValueError(f"owned perf child failed: {self.side}/{self.ordinal} exit={result.returncode}")
        if fixture_error is not None:
            raise ValueError("common fixture failed post-invocation full-file validation") from fixture_error
        if valgrind:
            chip = Path(command[command.index("--chip") + 1]).stem
            count = int(command[command.index("--max-steps") + 1])
            mode = "batch" if "--batched" in command else "step"
            single_step = environment.get("LABWIRED_ARM_SINGLE_STEP")
            if chip not in BOARDS or count not in COUNTS or (mode == "step" and single_step != "1") or (mode == "batch" and single_step is not None):
                raise ValueError("unexpected selected board/work/mode environment")
            ir = self.module.IREFS_RE.search(result.stderr.decode())
            proof = self.module.BATCHED_RE.search(result.stderr.decode())
            if not ir or (mode == "batch") != bool(proof):
                raise ValueError("missing instruction count or incorrect execution loop")
            same = [r for r in self.rows if (r["side"], r["board"], r["mode"], r["steps"]) == (self.side, chip, mode, count)]
            fixture = Path(command[command.index("--firmware") + 1])
            from cortex_m_oracle_evidence import verify_fixture
            fixture_proof = verify_fixture(fixture)
            cli = Path(command[command.index("run") - 1])
            self.rows.append(dict(side=self.side, board=chip, mode=mode, steps=count, repeat=len(same), exit_code=result.returncode,
                producer_exit_code=result.producer_exit_code, argv=argv, cwd=cwd,
                mode_environment={"LABWIRED_ARM_SINGLE_STEP": {"present": "LABWIRED_ARM_SINGLE_STEP" in environment, "value": single_step}},
                selected_flags=["--batched"] if mode == "batch" else [],
                ir=int(ir.group(1).replace(",", "")), instructions=int(proof.group(1)) if proof else None,
                batches=int(proof.group(2)) if proof else None, steps_per_batch=float(proof.group(3)) if proof else None,
                tick_interval=int(proof.group(4)) if proof else None, fixture_sha256=fixture_proof["sha256"],
                cli_sha256=sha256(cli), log_prefix=prefix.name))
            if self.common is not None:
                self.rows[-1].update(fixture_before_sha256=fixture_before["sha256"],fixture_after_sha256=fixture_after["sha256"])
                if self.raw_profiles is not None:
                    self.raw_profiles.capture(command,self.rows[-1])
                elif chip == "stm32l476" and mode == "batch":
                    from cortex_m_perf_diagnostics import annotate_call
                    self.rows[-1]["callgrind_annotation"] = annotate_call(command,self.rows[-1]["ir"],prefix,collect,self.budget,sha256)
        if kwargs.get("text"):
            result.stdout, result.stderr = result.stdout.decode(), result.stderr.decode()
        return result


def invoke_tool(module, source, cli, side, output, rows, budget, common=None, raw_profiles=None):
    original = module.subprocess, sys.argv[:], os.getcwd(), dict(os.environ)
    try:
        module.subprocess = BoundedRunner(module, side, output, rows, budget, common, raw_profiles)
        sys.argv = ["board_perf.py", "--boards", ",".join(BOARDS), "--modes", ",".join(MODES), "--cli", str(Path(cli).resolve()), "--status-json", str(Path(output).resolve() / f"{side}-status.json")]
        os.environ.pop("LABWIRED_ARM_SINGLE_STEP", None)
        os.chdir(source)
        with common.install(module) if common is not None else nullcontext():
            return module.main()
    finally:
        module.subprocess, sys.argv = original[0], original[1]
        os.chdir(original[2])
        os.environ.clear()
        os.environ.update(original[3])


def source_contract(source, expected):
    validate_sha(expected)
    actual = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=source, text=True).strip()
    if actual != expected:
        raise ValueError("checkout does not match immutable requested source")
    paths = ["Cargo.lock", "scripts/perf/board_perf.py", "scripts/perf/baselines.json", ".cargo/config.toml"] + [f"configs/chips/{b}.yaml" for b in BOARDS]
    paths += subprocess.check_output(["git", "ls-files", "crates/firmware-perf-spin"], cwd=source, text=True).splitlines()
    if not paths or not any(p.startswith("crates/firmware-perf-spin/") for p in paths):
        raise ValueError("missing performance fixture source")
    return {p: sha256(Path(source) / p) for p in paths}


def qualify(args):
    raw_mode = getattr(args,"diagnose_raw_l476_repeats",False)
    if raw_mode and (not getattr(args,"diagnose_common_fixtures",False) or args.suite != "cortex-m"):
        raise ValueError("raw L476 diagnosis requires common fixtures and Cortex-M suite")
    output = Path(args.output).resolve()
    output.mkdir(exist_ok=False)
    # Leave room under the job's 8MiB artifact bound for two build captures,
    # status JSON, metadata and the final receipt.
    budget = Budget(5 * 1024 * 1024)
    rows, codes, modules = [], {}, {}
    raw_profiles = None
    evidence = dict(scope="three-board Cortex-M A/B only; full CorePerf/RV1103 and silicon capture remain open", base_sha=args.base_sha, head_sha=args.head_sha)
    try:
        validate_sha(args.base_sha); validate_sha(args.head_sha)
        contracts = {side: source_contract(getattr(args, side + "_dir"), getattr(args, side + "_sha")) for side in SIDES}
        if contracts["base"] != contracts["head"]:
            raise ValueError("fixture/tool/config/lock/baseline contract bytes differ between sources")
        evidence["contracts"] = contracts
        evidence["identity"] = validate_identity(args.identity_dir, args.steps, args.base_sha, args.head_sha, args.suite)
        def load_side(side):
            source = Path(getattr(args, side + "_dir")).resolve()
            cli = source / "target/release/labwired"
            if not cli.is_file() or not os.access(cli, os.X_OK) or cli.stat().st_size == 0:
                raise ValueError("missing executable source-bound CLI")
            name = f"board_perf_{side}"
            spec = importlib.util.spec_from_file_location(name, source / "scripts/perf/board_perf.py")
            module = importlib.util.module_from_spec(spec)
            sys.modules[name] = module
            try:
                spec.loader.exec_module(module)
                modules[side] = module
                return module, source, cli
            except Exception:
                sys.modules.pop(name, None)
                raise
        common = None
        if getattr(args,"diagnose_common_fixtures",False):
            from cortex_m_perf_diagnostics import prepare_common_fixtures
            loaded = {side: load_side(side) for side in SIDES}
            cargo_contract = {side:sha256(Path(getattr(args,side+"_dir"))/"Cargo.toml") for side in SIDES}
            if len(set(cargo_contract.values())) != 1:
                raise ValueError("workspace Cargo profile bytes differ for common-input diagnostics")
            evidence["diagnostic_cargo_contract"] = cargo_contract
            common = prepare_common_fixtures(modules,{side:row[1] for side,row in loaded.items()},output,budget,BoundedRunner)
            evidence["common_fixtures"] = common.evidence
        if raw_mode:
            from cortex_m_raw_profiles import RawProfiles
            raw_profiles = RawProfiles(output,budget,collect,provenance=dict(
                base_sha=args.base_sha,head_sha=args.head_sha,contracts=contracts,
                common_fixtures={key:common.evidence[key] for key in ("common","originals")}))
        for side in SIDES:
            module,source,cli = loaded[side] if common is not None else load_side(side)
            try:
                codes[side] = invoke_tool(module,source,cli,side,output,rows,budget,common,raw_profiles)
            finally:
                sys.modules.pop(f"board_perf_{side}",None)
        if contracts != {side: source_contract(getattr(args, side + "_dir"), getattr(args, side + "_sha")) for side in SIDES}:
            raise ValueError("source contracts changed during qualification")
        if common is not None and cargo_contract != {side:sha256(Path(getattr(args,side+"_dir"))/"Cargo.toml") for side in SIDES}:
            raise ValueError("workspace Cargo profile changed during diagnostic qualification")
        if raw_profiles is not None:
            evidence["raw_profile_diagnosis"] = raw_profiles.finish()
        proof = validate_records(rows, modules["head"].MIN_RECOMMENDED_TICK_INTERVAL)
        evidence.update(proof)
        evidence["baseline_verdicts"] = {side: validate_status(small_json(output / f"{side}-status.json"), side, proof["measurements"], codes[side]) for side in SIDES}
        comparisons = []
        for board, mode in itertools.product(BOARDS, MODES):
            pair = [next(m["ir_per_step"] for m in proof["measurements"] if (m["side"], m["board"], m["mode"]) == (side, board, mode)) for side in SIDES]
            comparisons.append(dict(board=board, mode=mode, base=pair[0], head=pair[1], delta=(pair[1] - pair[0]) / pair[0], regression=modules["head"].is_regression(pair[1], pair[0], mode)))
        evidence["comparisons"] = comparisons
        if any(row["regression"] for row in comparisons):
            raise ValueError("source-change A/B regression under unchanged tool rule")
        evidence["qualified"] = True
        return 0
    except Exception as error:
        evidence["failure_reason"] = str(error)[:1024]
        raise
    finally:
        for side in modules:
            sys.modules.pop(f"board_perf_{side}",None)
        if raw_profiles is not None:
            evidence["raw_profile_diagnosis"] = raw_profiles.report
        evidence["raw_invocations"] = rows
        evidence.setdefault("qualified", False)
        data = json.dumps(evidence, indent=2, allow_nan=False) + "\n"
        if len(data.encode()) > 1024 * 1024:
            raise ValueError("oversized final qualification evidence")
        if raw_profiles is not None:
            try:
                raw_profiles.artifact_bytes(len(data.encode()))
            except (ValueError,OSError) as error:
                evidence["qualified"] = False
                evidence["artifact_failure_reason"] = str(error)[:1024]
                evidence.setdefault("failure_reason",str(error)[:1024])
                data=json.dumps(evidence,indent=2,allow_nan=False)+"\n"
                if len(data.encode())>1024*1024:raise ValueError("oversized final qualification evidence")
                (output / "qualification.json").write_text(data)
                raise
        (output / "qualification.json").write_text(data)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    collect_parser = sub.add_parser("collect")
    collect_parser.add_argument("--output", required=True)
    collect_parser.add_argument("--timeout", type=float, default=900)
    collect_parser.add_argument("producer", nargs=argparse.REMAINDER)
    q = sub.add_parser("qualify")
    q.add_argument("--diagnose-raw-l476-repeats",action="store_true",help="Retain three immutable base L476200k batch profiles without runtime annotation; requires common fixtures")
    q.add_argument("--diagnose-common-fixtures",action="store_true",help="Preserve independently built ELFs, verify/freeze common inputs and retain L476 Callgrind diagnostics")
    for name in ("base-dir", "head-dir", "base-sha", "head-sha", "identity-dir", "steps", "suite", "output"):
        q.add_argument("--" + name, required=True)
    args = parser.parse_args()
    try:
        if args.command == "qualify":
            return qualify(args)
        command = args.producer[1:] if args.producer[:1] == ["--"] else args.producer
        result = collect(command, timeout=args.timeout)
        prefix = Path(args.output)
        prefix.with_suffix(".stdout").write_bytes(result.stdout)
        prefix.with_suffix(".stderr").write_bytes(result.stderr)
        prefix.with_suffix(".json").write_text(json.dumps(dict(exit_code=result.returncode, producer_exit_code=result.producer_exit_code)) + "\n")
        return result.returncode if result.returncode >= 0 else 128 - result.returncode
    except (ValueError, OSError, KeyError, TypeError) as error:
        print(f"FAIL-PERF-EVIDENCE {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
