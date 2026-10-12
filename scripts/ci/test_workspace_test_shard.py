# LabWired - Firmware Simulation Platform
# Copyright (C) 2026 Andrii Shylenko
#
# This software is released under the MIT License.
# See the LICENSE file in the project root for full license information.
#
# Unit tests for the pr-workspace-tests shard driver and aggregator. These
# cover the PARSING and the CLASSIFICATION only — no cargo invocation — because
# a gate that mis-reads libtest output is the same false-green class the gate
# itself exists to close. Wired into pr-gate's pytest line in core-ci.yml.

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent))

import workspace_test_aggregate as agg
import workspace_test_shard as shard


# ── libtest output parsing ───────────────────────────────────────────────────

PASSING = """\
   Compiling labwired-core v0.21.0 (/w/crates/core)
    Finished `test` profile [optimized + debuginfo] target(s) in 0.12s
     Running tests/foo.rs (/w/target/debug/deps/foo-1234)

running 3 tests
test alpha ... ok
test beta ... ok
test gamma ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.25s
"""

FAILING = """\
     Running tests/env.rs (/w/target/debug/deps/env-5678)

running 2 tests
test keeps_working ... ok
test prioritizes_failed_assertions ... FAILED

failures:

---- prioritizes_failed_assertions stdout ----
thread 'prioritizes_failed_assertions' (12296) panicked at crates/cli/tests/env.rs:1394:5:
assertion `left == right` failed

failures:
    prioritizes_failed_assertions

test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.56s

error: test failed, to rerun pass `-p labwired-cli --test env`
"""

VACUOUS = """\
     Running tests/empty.rs (/w/target/debug/deps/empty-0000)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
"""

ALL_IGNORED = """\
     Running tests/bench.rs (/w/target/debug/deps/bench-0000)

running 2 tests
test bench_a ... ignored
test bench_b ... ignored

test result: ok. 0 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 0.00s
"""

WITH_SKIP = """\
     Running tests/world.rs (/w/target/debug/deps/world-0000)

running 1 tests
SKIP: iolink ELFs not built; build it: make -C examples/iolink-dido/firmware
test station_boots ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.40s
"""


def test_parse_counts_and_duration():
    p = shard.parse_run(PASSING)
    assert (p["passed"], p["failed"], p["ignored"]) == (3, 0, 0)
    assert p["result_lines"] == 1
    assert p["duration_s"] == 1.25


def test_parse_failed_test_names_from_failed_lines():
    p = shard.parse_run(FAILING)
    assert p["failed"] == 1
    assert p["failed_tests"] == ["prioritizes_failed_assertions"]


def test_parse_skip_channel_is_visible_not_silent():
    p = shard.parse_run(WITH_SKIP)
    assert p["passed"] == 1
    assert any("SKIP:" in s for s in p["skips"])


def test_vacuous_and_all_ignored_are_distinguishable():
    # The vacuity contract: 0/0/0/0 is the empty-binary shape; ignored > 0 is a
    # suite that legitimately ran nothing.
    v = shard.parse_run(VACUOUS)
    assert (v["passed"], v["failed"], v["ignored"], v["filtered"]) == (0, 0, 0, 0)
    i = shard.parse_run(ALL_IGNORED)
    assert i["ignored"] == 2 and i["passed"] == 0


# ── shard assignment ─────────────────────────────────────────────────────────

def test_round_robin_covers_everything_exactly_once():
    runnable = [(f"pkg{c}", f"t{i}", "test") for i in range(10) for c in "ab"]
    runnable.sort()
    slices = [shard.shard_slice(runnable, k, 3) for k in (1, 2, 3)]
    flat = [e for s in slices for e in s]
    assert sorted(flat) == runnable
    # deterministic: same input, same split
    assert slices == [shard.shard_slice(runnable, k, 3) for k in (1, 2, 3)]


def _entry(pkg, tgt, kind="test"):
    return {"package": pkg, "target": tgt, "kind": kind, "exe": None, "cwd": "."}


def test_classification_rejects_stale_exclusion():
    cfg = {"shard_count": 3, "cross_build_excluded": [
        {"package": "p", "target": "ghost", "needs": "x", "reason": "y"}],
        "known_red": []}
    _, problems = shard.classify(cfg, [_entry("p", "real")])
    assert any("ghost" in p for p in problems)


def test_classification_rejects_known_red_without_a_target():
    cfg = {"shard_count": 3, "cross_build_excluded": [], "known_red": [
        {"package": "p", "target": "ghost", "test": "t", "issue": 1, "reason": "r"}]}
    _, problems = shard.classify(cfg, [_entry("p", "real")])
    assert any("ghost" in p for p in problems)


# ── cargo test's runtime environment, reproduced ─────────────────────────────

def test_test_env_sets_manifest_dir_and_bin_exes(tmp_path):
    exe = tmp_path / "debug" / "deps" / "cli_integration-abc"
    exe.parent.mkdir(parents=True)
    exe.touch()
    entry = {
        "package": "labwired-cli", "package_id": "pkg-cli",
        "target": "cli_integration", "kind": "test",
        "exe": str(exe), "cwd": "/w/crates/cli",
    }
    env = shard.test_env(entry, {"pkg-cli": [("labwired", "/w/target/debug/labwired")]})
    assert env["CARGO_MANIFEST_DIR"] == "/w/crates/cli"
    assert env["CARGO_BIN_EXE_labwired"] == "/w/target/debug/labwired"
    # tmpdir's parent resolves to the target dir — test_support's contract.
    assert Path(env["CARGO_TARGET_TMPDIR"]).parent == tmp_path
    assert Path(env["CARGO_TARGET_TMPDIR"]).is_dir()


def test_test_env_mangles_dashed_bin_names(tmp_path):
    exe = tmp_path / "debug" / "deps" / "t-1"
    exe.parent.mkdir(parents=True)
    exe.touch()
    entry = {"package": "p", "package_id": "p1", "target": "t", "kind": "test",
             "exe": str(exe), "cwd": "/w/p"}
    env = shard.test_env(entry, {"p1": [("my-tool", "/b/my-tool")],
                                 "other": [("nope", "/b/nope")]})
    assert env["CARGO_BIN_EXE_my_tool"] == "/b/my-tool"
    assert "CARGO_BIN_EXE_nope" not in env  # only same-package bins


# ── aggregation verdict ──────────────────────────────────────────────────────

def _write_report(root, k, targets, new_red=None, known_red_seen=None, hard=None):
    d = Path(root) / f"workspace-test-shard-{k}"
    d.mkdir(parents=True, exist_ok=True)
    (d / f"shard-{k}.json").write_text(json.dumps({
        "shard": k,
        "shard_count": 3,
        "targets": targets,
        "hard_errors": hard or [],
        "new_red": new_red or [],
        "known_red_seen": known_red_seen or [],
    }))


def _target(pkg, tgt, status="pass", failed_tests=None, dur=1.0):
    return {
        "package": pkg, "target": tgt, "kind": "test", "status": status,
        "passed": 5 if status == "pass" else 4,
        "failed": 0 if status == "pass" else 1,
        "ignored": 0, "filtered": 0, "duration_s": dur,
        "failed_tests": failed_tests or [], "skips": [],
    }


def _cfg(known_red=None):
    return {"shard_count": 3, "cross_build_excluded": [], "known_red": known_red or []}


def test_missing_shard_report_is_a_failure_not_a_pass(tmp_path):
    _write_report(tmp_path, 1, [_target("p", "a")])
    _write_report(tmp_path, 2, [_target("p", "b")])
    failures, _, _, _ = agg.aggregate(tmp_path, _cfg())
    assert any("shard 3" in f and "NO REPORT" in f for f in failures)


def test_new_red_fails_and_names_the_test(tmp_path):
    for k in (1, 2, 3):
        _write_report(tmp_path, k, [_target("p", f"t{k}")])
    _write_report(tmp_path, 4, [])  # ignored: only shards 1..3 are expected
    failures, _, _, _ = agg.aggregate(
        tmp_path,
        _cfg(),
    )
    assert failures == []

    _write_report(tmp_path, 2, [_target("p", "t2")], new_red=[
        {"package": "p", "target": "t2", "test": "broke"}])
    failures, _, _, _ = agg.aggregate(tmp_path, _cfg())
    assert any("NEW RED" in f and "broke" in f for f in failures)


def test_known_red_still_failing_is_a_notice_not_a_failure(tmp_path):
    entry = {"package": "p", "target": "t1", "test": "flaky", "issue": 42, "reason": "r"}
    for k in (1, 2, 3):
        _write_report(tmp_path, k, [_target("p", f"t{k}")])
    _write_report(tmp_path, 1, [
        _target("p", "t1", status="fail", failed_tests=["flaky"])],
        known_red_seen=[{**entry}])
    failures, notices, _, _ = agg.aggregate(tmp_path, _cfg([entry]))
    assert failures == []
    assert any("#42" in n for n in notices)


def test_known_red_now_passing_is_stale_and_fails(tmp_path):
    # The ratchet that shrinks the allow-list: a fixed test must not keep its
    # entry, or the entry silently re-arms and swallows the next regression.
    entry = {"package": "p", "target": "t1", "test": "fixed", "issue": 42, "reason": "r"}
    for k in (1, 2, 3):
        _write_report(tmp_path, k, [_target("p", f"t{k}")])
    failures, _, _, _ = agg.aggregate(tmp_path, _cfg([entry]))
    assert any("STALE" in f and "#42" in f for f in failures)


def test_known_red_whose_target_never_ran_fails(tmp_path):
    entry = {"package": "p", "target": "ghost", "test": "t", "issue": 7, "reason": "r"}
    for k in (1, 2, 3):
        _write_report(tmp_path, k, [_target("p", f"t{k}")])
    failures, _, _, _ = agg.aggregate(tmp_path, _cfg([entry]))
    assert any("ghost" in f and "did not run" in f for f in failures)


# Literal nonvacuous-wrapper subprocess controls. Only the owned fake cargo and
# fake libtest executable run: no Rust compiler, package install or firmware.
@pytest.fixture
def nonvacuous_fixture(tmp_path):
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    artifacts = tmp_path / "artifacts"
    artifacts.mkdir()
    calls = tmp_path / "cargo-calls.jsonl"
    lists = tmp_path / "list-calls.jsonl"
    exe = tmp_path / "metadata-fixture-0123456789abcdef"
    exe.write_text(f"#!{sys.executable}\n" + r'''
import json, os, sys
with open(os.environ["FAKE_LIST_CALLS"], "a") as fh:
    fh.write(json.dumps(sys.argv[1:]) + "\n")
count = int(os.environ["FAKE_IGNORED"] if "--ignored" in sys.argv else os.environ["FAKE_DECLARED"])
for index in range(count):
    print(f"fixture_{index}: test")
''')
    exe.chmod(0o700)
    cargo = bin_dir / "cargo"
    cargo.write_text(f"#!{sys.executable}\n" + r'''
import json, os, sys
with open(os.environ["FAKE_CARGO_CALLS"], "a") as fh:
    fh.write(json.dumps(sys.argv[1:]) + "\n")
if "--no-run" in sys.argv:
    sys.stdout.write(os.environ["FAKE_MESSAGES"])
    sys.stderr.write("original cargo stderr\n")
    sys.exit(int(os.environ["FAKE_COMPILE_STATUS"]))
print("fake real test execution")
sys.exit(int(os.environ["FAKE_RUN_STATUS"]))
''')
    cargo.chmod(0o700)
    wrapper = Path(__file__).with_name("cargo-test-nonvacuous.sh")

    def invoke(*, messages=None, compile_status=0, run_status=0, declared=2, ignored=0,
               harness=("--nocapture",), reader_failure=False):
        if messages is None:
            messages = json.dumps({"reason": "compiler-artifact", "profile": {"test": True},
                                   "executable": str(exe)}) + "\n"
        if reader_failure:
            reader = bin_dir / "python3"
            reader.write_text("#!/usr/bin/env bash\necho 'owned diagnostic reader failed' >&2\nexit 7\n")
            reader.chmod(0o700)
        env = {**os.environ, "PATH": str(bin_dir) + os.pathsep + os.environ.get("PATH", ""),
               "TMPDIR": str(artifacts), "GITHUB_STEP_SUMMARY": "",
               "FAKE_CARGO_CALLS": str(calls), "FAKE_LIST_CALLS": str(lists),
               "FAKE_MESSAGES": messages, "FAKE_COMPILE_STATUS": str(compile_status),
               "FAKE_RUN_STATUS": str(run_status), "FAKE_DECLARED": str(declared),
               "FAKE_IGNORED": str(ignored)}
        args = ["--locked", "-p", "metadata-fixture", "--test", "ordered_debug_metadata"]
        result = subprocess.run(["bash", str(wrapper), *args, "--", *harness], env=env,
                                capture_output=True, text=True, timeout=5)
        read = lambda path: [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []
        return result, read(calls), read(lists), list(artifacts.iterdir()), args

    return invoke


def test_nonvacuous_compile_failure_retains_original_rendered_error_and_status(nonvacuous_fixture):
    rendered = "\x1b[1merror[E0308]: mismatched types\x1b[0m\n --> ordered_debug_metadata.rs:25:50\n expected BTreeMap, found HashMap; Ω\n"
    messages = json.dumps({"reason": "compiler-message", "message": {"level": "error", "rendered": rendered}}) + "\n"
    result, calls, lists, artifacts, args = nonvacuous_fixture(messages=messages, compile_status=101)
    assert result.returncode == 101
    assert result.stderr == "original cargo stderr\n" + rendered
    assert calls == [["test", *args, "--no-run", "--message-format=json"]]
    assert lists == []
    assert artifacts == []


@pytest.mark.parametrize("status", [17, 130, 143])
def test_nonvacuous_diagnostics_never_replace_compile_failure_status(nonvacuous_fixture, status):
    result, calls, lists, artifacts, _ = nonvacuous_fixture(messages="not JSON\n", compile_status=status)
    assert result.returncode == status
    assert result.stderr == "original cargo stderr\n"
    assert len(calls) == 1 and lists == [] and artifacts == []


def test_nonvacuous_success_preserves_enumeration_roster_and_final_argv(nonvacuous_fixture):
    result, calls, lists, _, args = nonvacuous_fixture()
    assert result.returncode == 0
    assert calls == [["test", *args, "--no-run", "--message-format=json"],
                     ["test", *args, "--", "--nocapture"]]
    assert lists == [["--list", "--format", "terse"], ["--list", "--ignored", "--format", "terse"]]
    assert "2 declared, 2 executed by this invocation (0 ignored, run mode: default)" in result.stdout
    assert "fake real test execution" in result.stdout
    assert result.stderr == "original cargo stderr\n"


@pytest.mark.parametrize("declared,ignored,phrase", [(0, 0, "contain ZERO tests"), (2, 2, "EXECUTE NONE")])
def test_nonvacuous_empty_and_allignored_still_reject_before_execution(nonvacuous_fixture, declared, ignored, phrase):
    result, calls, lists, artifacts, _ = nonvacuous_fixture(declared=declared, ignored=ignored)
    assert result.returncode == 1
    assert phrase in result.stderr
    assert len(calls) == 1 and len(lists) == 2 and artifacts == []


def test_nonvacuous_missing_executable_still_fails_closed(nonvacuous_fixture):
    result, calls, lists, artifacts, _ = nonvacuous_fixture(messages="{}\n")
    assert result.returncode == 1
    assert "NO test executables" in result.stderr
    assert len(calls) == 1 and lists == [] and artifacts == []


def test_nonvacuous_success_still_propagates_final_test_status(nonvacuous_fixture):
    result, calls, _, _, _ = nonvacuous_fixture(run_status=23)
    assert result.returncode == 23
    assert len(calls) == 2



def test_nonvacuous_only_original_rendered_errors_are_emitted_in_order(nonvacuous_fixture):
    messages = [[], None, {"reason": "compiler-message", "message": []},
                {"reason": "compiler-message", "message": {"level": "warning", "rendered": "warning payload"}},
                {"reason": "compiler-artifact", "message": {"level": "error", "rendered": "unrelated payload"}},
                {"reason": "compiler-message", "message": {"level": "error", "rendered": None}},
                {"reason": "compiler-message", "message": {"level": "error", "rendered": "first original error\n"}},
                {"reason": "compiler-message", "message": {"level": "error", "rendered": "second original error\n"}}]
    text = "invalid JSON\n" + "\n".join(json.dumps(msg) for msg in messages) + "\n"
    result, calls, lists, artifacts, _ = nonvacuous_fixture(messages=text, compile_status=19)
    assert result.returncode == 19
    assert result.stderr == "original cargo stderr\nfirst original error\nsecond original error\n"
    assert len(calls) == 1 and lists == [] and artifacts == []


def test_nonvacuous_reader_failure_never_masks_original_cargo_failure(nonvacuous_fixture):
    result, calls, lists, artifacts, _ = nonvacuous_fixture(messages="{}\n", compile_status=41, reader_failure=True)
    assert result.returncode == 41
    assert "owned diagnostic reader failed" in result.stderr
    assert len(calls) == 1 and lists == [] and artifacts == []


@pytest.mark.parametrize("harness,executed,mode", [(("--ignored",), 1, "ignored"),
                        (("--include-ignored",), 2, "include-ignored"),
                        (("--ignored", "--include-ignored"), 2, "include-ignored")])
def test_nonvacuous_success_preserves_harness_modes(nonvacuous_fixture, harness, executed, mode):
    result, calls, _, _, args = nonvacuous_fixture(declared=2, ignored=1, harness=harness)
    assert result.returncode == 0
    assert calls[-1] == ["test", *args, "--", *harness]
    assert f"2 declared, {executed} executed by this invocation (1 ignored, run mode: {mode})" in result.stdout



def test_nonvacuous_original_internal_compiler_error_is_also_retained(nonvacuous_fixture):
    rendered = "error: internal compiler error: original compiler report\n"
    messages = json.dumps({"reason": "compiler-message", "message": {
        "level": "error: internal compiler error", "rendered": rendered}}) + "\n"
    result, calls, lists, artifacts, _ = nonvacuous_fixture(messages=messages, compile_status=101)
    assert result.returncode == 101 and result.stderr == "original cargo stderr\n" + rendered
    assert len(calls) == 1 and lists == [] and artifacts == []
