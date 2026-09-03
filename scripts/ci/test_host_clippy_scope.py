"""Unit tests for the derived host-clippy scope.

The defect this scope closes is a gate that looks like it lints a workspace and
lints five packages of eighty-four. A test suite for it therefore has to assert
two different things: that the classifier puts each SHAPE of crate where it
belongs (synthetic trees, below), and that on the LIVE tree it is not passing
vacuously — a classifier that called everything host-lintable, or everything
firmware, would satisfy every synthetic case here and gate nothing.
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent))

import host_clippy_scope as scope  # noqa: E402


def make_repo(tmp_path: Path, crates: dict[str, dict], *, default_members: list[str] | None = None,
              workflow: str | None = None) -> Path:
    """Build a synthetic workspace.

    `crates` maps a member path to a dict of: `name`, optional `manifest_extra`
    (raw TOML appended to Cargo.toml), optional `files` (relative path -> text),
    optional `cargo_target` (written to the crate's .cargo/config.toml).
    """
    tmp_path.mkdir(parents=True, exist_ok=True)
    members = list(crates)
    default = default_members if default_members is not None else members
    root_toml = [
        "[workspace]",
        "resolver = \"2\"",
        "members = [" + ", ".join(f'"{m}"' for m in members) + "]",
        "default-members = [" + ", ".join(f'"{m}"' for m in default) + "]",
    ]
    (tmp_path / "Cargo.toml").write_text("\n".join(root_toml) + "\n", encoding="utf-8")

    for member, spec in crates.items():
        crate = tmp_path / member
        (crate / "src").mkdir(parents=True, exist_ok=True)
        manifest = f'[package]\nname = "{spec["name"]}"\nversion = "0.0.0"\nedition = "2021"\n'
        manifest += spec.get("manifest_extra", "")
        (crate / "Cargo.toml").write_text(manifest, encoding="utf-8")
        for rel, text in (spec.get("files") or {"src/lib.rs": "pub fn f() {}\n"}).items():
            path = crate / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding="utf-8")
        if spec.get("cargo_target"):
            cfg = crate / ".cargo"
            cfg.mkdir(parents=True, exist_ok=True)
            (cfg / "config.toml").write_text(
                f'[build]\ntarget = "{spec["cargo_target"]}"\n', encoding="utf-8"
            )

    if workflow is not None:
        wf = tmp_path / ".github" / "workflows"
        wf.mkdir(parents=True)
        (wf / "ci.yml").write_text(workflow, encoding="utf-8")
    return tmp_path


HOST = {"name": "host-crate"}
NOSTD = {"name": "fw-crate", "files": {"src/main.rs": "#![no_std]\n#![no_main]\nfn main() {}\n"}}


def names(entries: list[dict]) -> set[str]:
    return {e["package"] for e in entries}


# ── the two signals ───────────────────────────────────────────────────────────


def test_a_plain_host_crate_is_lintable(tmp_path):
    host, cross = scope.classify(make_repo(tmp_path, {"crates/a": HOST}))
    assert names(host) == {"host-crate"}
    assert cross == []


def test_no_std_marks_a_crate_cross_target(tmp_path):
    host, cross = scope.classify(make_repo(tmp_path, {"crates/a": NOSTD}))
    assert host == []
    assert names(cross) == {"fw-crate"}
    assert "no_std" in cross[0]["reason"]


def test_conditional_no_std_counts_too(tmp_path):
    """`#![cfg_attr(not(test), no_std)]` — the live spelling in obd2-scanner.

    A crate that is no_std under any configuration is firmware. Reading only
    the bare attribute would call that one a host crate.
    """
    crate = {"name": "fw", "files": {"src/lib.rs": "#![cfg_attr(not(test), no_std)]\n"}}
    host, cross = scope.classify(make_repo(tmp_path, {"crates/a": crate}))
    assert host == []
    assert names(cross) == {"fw"}


@pytest.mark.parametrize(
    "triple",
    [
        "thumbv6m-none-eabi",
        "thumbv7em-none-eabi",
        "thumbv8m.main-none-eabi",
        "riscv32imac-unknown-none-elf",
        "xtensa-esp32s3-none-elf",
        "wasm32-unknown-unknown",
    ],
)
def test_a_pinned_bare_metal_target_marks_a_crate_cross_target(tmp_path, triple):
    crate = {"name": "fw", "cargo_target": triple}
    host, cross = scope.classify(make_repo(tmp_path, {"crates/a": crate}))
    assert host == []
    assert triple in cross[0]["reason"]


def test_a_cross_target_crate_that_is_not_no_std_is_still_caught(tmp_path):
    """firmware-f407-demo is exactly this: pinned target, no `#![no_std]` root.

    Its two `[[bin]]` targets live at src/smoke.rs and src/i2c.rs, so the
    default-path scan finds nothing to read. The pinned target is what saves it.
    """
    crate = {
        "name": "fw",
        "cargo_target": "thumbv7em-none-eabi",
        "manifest_extra": '\n[[bin]]\nname = "smoke"\npath = "src/smoke.rs"\n',
        "files": {"src/smoke.rs": "#![no_std]\n#![no_main]\n"},
    }
    host, cross = scope.classify(make_repo(tmp_path, {"crates/a": crate}))
    assert host == []


def test_declared_bin_paths_are_read_for_no_std(tmp_path):
    """The same crate WITHOUT the pinned target still classifies correctly.

    Two independent signals, and either one alone is enough — which is what
    makes a crate that drops its .cargo/config.toml not silently become a host
    crate.
    """
    crate = {
        "name": "fw",
        "manifest_extra": '\n[[bin]]\nname = "smoke"\npath = "src/smoke.rs"\n',
        "files": {"src/smoke.rs": "#![no_std]\n#![no_main]\n"},
    }
    host, cross = scope.classify(make_repo(tmp_path, {"crates/a": crate}))
    assert host == []
    assert "src/smoke.rs" in cross[0]["reason"]


def test_a_custom_lib_path_is_read(tmp_path):
    crate = {
        "name": "fw",
        "manifest_extra": '\n[lib]\npath = "src/other.rs"\n',
        "files": {"src/other.rs": "#![no_std]\n"},
    }
    _, cross = scope.classify(make_repo(tmp_path, {"crates/a": crate}))
    assert names(cross) == {"fw"}


# ── failing closed ────────────────────────────────────────────────────────────


def test_an_unrecognised_pinned_target_is_an_error_not_a_guess(tmp_path):
    """A pinned host-looking triple must stop the gate, not fall through.

    Either answer would be a guess, and the wrong guess drops a crate out of
    clippy with nothing to show for it — the defect this file exists to prevent.
    """
    crate = {"name": "odd", "cargo_target": "x86_64-unknown-linux-musl"}
    root = make_repo(tmp_path, {"crates/a": crate})
    with pytest.raises(scope.Unclassified) as err:
        scope.classify(root)
    assert "x86_64-unknown-linux-musl" in str(err.value)
    assert scope.main(["--check", "--root", str(root)]) == 1


def test_a_member_with_no_crate_root_is_an_error(tmp_path):
    crate = {"name": "empty", "files": {"README.md": "nothing here\n"}}
    with pytest.raises(scope.Unclassified):
        scope.classify(make_repo(tmp_path, {"crates/a": crate}))


# ── the soundness checks --check runs ─────────────────────────────────────────


# A workflow that covers the other two thirds of the partition: the bare
# default-members command, and the dedicated browser-layer step.
WASM_STEP = (
    "        run: cargo clippy --all-targets -- -D warnings\n"
    "        run: cargo clippy -p labwired-wasm --all-targets -- -D warnings\n"
)

# A three-crate workspace shaped like the real one: one default-member that the
# bare command covers, one host crate outside it that this scope must emit, one
# firmware crate, and the excused browser package.
TRIO = {
    "crates/d": {"name": "default-member"},
    "crates/a": HOST,
    "crates/b": NOSTD,
    "crates/w": {"name": "labwired-wasm"},
}
DEFAULTS = ["crates/d"]


def test_a_tree_with_only_host_crates_fails_the_check(tmp_path):
    """No cross-target class means the classifier discriminated nothing."""
    root = make_repo(tmp_path, {"crates/a": HOST}, workflow=WASM_STEP)
    assert any("cross-target-only" in p for p in scope.problems(root))


def test_a_tree_with_only_firmware_fails_the_check(tmp_path):
    root = make_repo(tmp_path, {"crates/a": NOSTD}, workflow=WASM_STEP)
    assert any("empty" in p for p in scope.problems(root))


def test_a_scope_that_emits_nothing_fails_the_check(tmp_path):
    """A step that lints no package reports green for doing no work.

    Exactly the shape of the bug being fixed: a command that looks like it
    covers a workspace and covers a subset — here, the empty one.
    """
    crates = {"crates/d": {"name": "default-member"}, "crates/b": NOSTD,
              "crates/w": {"name": "labwired-wasm"}}
    root = make_repo(tmp_path, crates, default_members=DEFAULTS, workflow=WASM_STEP)
    assert scope.clippy_packages(root) == []
    assert any("lints nothing" in p for p in scope.problems(root))


def test_a_default_member_that_classifies_as_firmware_is_a_contradiction(tmp_path):
    """`cargo clippy --all-targets` builds every default-member for the host.

    So a default-member the classifier calls cross-target-only means the
    classifier is wrong, and saying so beats silently shrinking the scope.
    """
    root = make_repo(
        tmp_path, TRIO, default_members=["crates/d", "crates/b"], workflow=WASM_STEP
    )
    assert any("default-member `crates/b`" in p for p in scope.problems(root))


def test_the_reference_tree_is_sound(tmp_path):
    """The positive control for every negative one below it."""
    root = make_repo(tmp_path, TRIO, default_members=DEFAULTS, workflow=WASM_STEP)
    assert scope.problems(root) == []


def test_default_members_are_left_to_the_bare_command(tmp_path):
    """This scope is the COMPLEMENT, not a second copy.

    Re-linting them here would double the step's cost for no coverage — the
    measured reason the crates/wasm steps were split out of pr-gate in the
    first place.
    """
    root = make_repo(tmp_path, TRIO, default_members=DEFAULTS, workflow=WASM_STEP)
    assert scope.clippy_packages(root) == ["host-crate"]


def test_a_bare_clippy_step_must_still_exist_somewhere(tmp_path):
    """Narrow every `cargo clippy` to a `-p` list and default-members go dark.

    This scope subtracts them on the strength of that command existing. If it
    stops existing, the subtraction stops being safe, and saying so is the
    difference between a partition and a hole.
    """
    root = make_repo(
        tmp_path,
        TRIO,
        default_members=DEFAULTS,
        workflow="        run: cargo clippy -p labwired-wasm --all-targets -- -D warnings\n",
    )
    assert any("runs without `-p`" in p for p in scope.problems(root))


def test_the_excused_package_must_still_be_linted_somewhere(tmp_path):
    """Delete the browser-layer clippy step and this scope must notice.

    An exception whose justification has quietly evaporated is the same hole
    with better paperwork.
    """
    root = make_repo(
        tmp_path,
        TRIO,
        default_members=DEFAULTS,
        workflow="        run: cargo clippy --all-targets -- -D warnings\n",
    )
    assert any("linted by nothing" in p for p in scope.problems(root))


def test_a_commented_out_clippy_step_does_not_cover_the_exception(tmp_path):
    root = make_repo(
        tmp_path,
        TRIO,
        default_members=DEFAULTS,
        workflow=(
            "        run: cargo clippy --all-targets -- -D warnings\n"
            "        # run: cargo clippy -p labwired-wasm --all-targets\n"
        ),
    )
    assert any("linted by nothing" in p for p in scope.problems(root))


def test_cargo_args_are_one_package_per_line(tmp_path, capsys):
    root = make_repo(tmp_path, TRIO, default_members=DEFAULTS, workflow=WASM_STEP)
    assert scope.main(["--cargo-args", "--root", str(root)]) == 0
    assert capsys.readouterr().out.splitlines() == ["--package=host-crate"]


# ── the live tree ─────────────────────────────────────────────────────────────


def test_this_repository_passes():
    assert scope.problems(scope.REPO_ROOT) == []


def test_the_live_tree_has_crates_in_BOTH_classes():
    """The non-vacuity assertion.

    A classifier that answered "host" for everything, or "firmware" for
    everything, passes every synthetic case above. This is the one that fails.
    """
    host, cross = scope.classify(scope.REPO_ROOT)
    assert len(host) > 1
    assert len(cross) > 1
    assert len(host) + len(cross) == len(scope.workspace(scope.REPO_ROOT)["members"])


def test_the_scope_is_strictly_wider_than_the_default_members():
    """The whole point: `cargo clippy --all-targets` lints default-members only.

    If this ever stops holding, the new step has stopped adding coverage and the
    hole is back with a script in front of it.
    """
    default = set(scope.workspace(scope.REPO_ROOT)["default-members"])
    host, _ = scope.classify(scope.REPO_ROOT)
    host_members = {e["member"] for e in host}
    assert default < host_members


def test_the_crates_this_change_was_written_for_are_in_scope():
    """Named on purpose, and NOT as the source of the scope.

    The script derives its scope from the tree; this test asserts the derivation
    actually reaches the crates that motivated it, which a rule that quietly
    narrowed would not.

    The two groups are different defects and both are real. NO_CLIPPY_AT_ALL is
    reached by no lane in any target. VIA_DEPENDENCY_ONLY had its *lib* linted
    incidentally — cargo runs clippy-driver over workspace members it builds as
    dependencies too — but never its tests, benches, bins or `#[cfg(test)]`
    modules, which is the half `--all-targets` exists for.
    """
    NO_CLIPPY_AT_ALL = {
        "labwired-egress-relay",
        "labwired-hw-oracle",
        "labwired-hw-oracle-macros",
        "labwired-hw-runner",
        "validation-report",
        "labwired-python",
    }
    VIA_DEPENDENCY_ONLY = {
        "labwired-codegen",
        "labwired-config",
        "labwired-gdbstub",
        "labwired-hw-trace",
        "labwired-ir",
        "svd-ingestor",
    }
    linted = set(scope.clippy_packages(scope.REPO_ROOT))
    assert NO_CLIPPY_AT_ALL <= linted
    assert VIA_DEPENDENCY_ONLY <= linted


def test_no_firmware_crate_leaks_into_the_scope():
    """Firmware in the scope would fail the step for the wrong reason.

    `cargo clippy -p firmware-rp2040-demo` from the workspace root builds it for
    the HOST — its own .cargo/config.toml is out of cargo's config search path
    from there — and a `#![no_std]` Cortex-M crate does not link on x86.
    """
    linted = scope.clippy_packages(scope.REPO_ROOT)
    assert [n for n in linted if n.startswith("firmware-")] == []
    assert [n for n in linted if n.endswith("-lab")] == []
