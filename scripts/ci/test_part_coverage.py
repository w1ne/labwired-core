"""Unit cover for the part-coverage gate.

Every case builds a SYNTHETIC tree and points the gate at it with
`--repo-root`. Grading the real checkout here would make these tests a mirror
of today's tree, and could never express "a part nothing exercises must fail",
because the real tree is (by design) not in that state.

The first two cases are the NEGATIVE CONTROL: a part with no lab, no test and
no ledger entry must fail by name, with or without a ledger file. Without them
a gate that derived "exercised" for everything would look identical to one
that works. The last case grades the committed tree.
"""

import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
GATE = REPO_ROOT / "scripts/ci/part_coverage.py"


def build_tree(root: Path, *, parts, labs=(), tests=(), ledger=None):
    """Write the smallest tree the gate can grade.

    parts: descriptor stems; each gets a file and an EMBEDDED_DEVICES row
           answering to the stem (a (stem, [spellings]) pair overrides that).
    labs:  (lab dir, [types]) — an example lab whose system.yaml attaches them.
    tests: (relative path, text) — integration test sources.
    """
    (root / "configs/devices").mkdir(parents=True)
    (root / "crates/config/src").mkdir(parents=True)
    rows = []
    for part in parts:
        stem, names = part if isinstance(part, tuple) else (part, [part])
        (root / f"configs/devices/{stem}.yaml").write_text(f"type: {names[0]}\n")
        quoted = ", ".join(f'"{n}"' for n in names)
        rows.append(
            f'    (\n        &[{quoted}],\n'
            f'        include_str!("../../../configs/devices/{stem}.yaml"),\n    ),\n'
        )
    (root / "crates/config/src/embedded.rs").write_text(
        "pub static EMBEDDED_DEVICES: &[(&[&str], &str)] = &[\n" + "".join(rows) + "];\n"
    )

    (root / "examples").mkdir()
    for lab, types in labs:
        (root / "examples" / lab).mkdir(parents=True)
        devices = "".join(
            f'  - id: "d{i}"\n    type: "{t}"\n    connection: "i2c1"\n'
            for i, t in enumerate(types)
        )
        (root / "examples" / lab / "system.yaml").write_text(
            f'chip: "stm32f103"\nexternal_devices:\n{devices}'
        )

    for rel, text in tests:
        path = root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    (root / "configs/ci").mkdir(parents=True)
    if ledger is not None:
        (root / "configs/ci/part-coverage.yaml").write_text(ledger)


def run_gate(root: Path):
    proc = subprocess.run(
        [sys.executable, str(GATE), "--repo-root", str(root)],
        capture_output=True,
        text=True,
        check=False,
    )
    return proc.returncode, proc.stdout + proc.stderr


def ledger(*, device_only=0, lab=0, parts=""):
    return f"max_device_only: {device_only}\nmin_lab_exercised: {lab}\nparts:\n{parts or '  {}'}\n"


# ── negative control ──────────────────────────────────────────────────────


def test_a_part_with_no_lab_no_test_and_no_exception_is_a_gap(tmp_path):
    """THE negative control: silence must not pass, and the message names the part."""
    build_tree(
        tmp_path,
        parts=["tmp102", "orphan9000"],
        labs=[("tmp102-lab", ["tmp102"])],
        ledger=ledger(lab=1),
    )
    code, out = run_gate(tmp_path)
    assert code == 1, out
    assert "part 'orphan9000'" in out and "NOTHING exercises it" in out
    assert "kind: device-only" in out, "the message must say what to do"
    assert "1 gap" in out


def test_no_ledger_file_fails_closed_on_a_gap(tmp_path):
    """Deleting the ledger must make the gate stricter, not quieter."""
    build_tree(tmp_path, parts=["orphan9000"], ledger=None)
    code, out = run_gate(tmp_path)
    assert code == 1, out
    assert "orphan9000" in out


# ── what counts as exercised ──────────────────────────────────────────────


def test_a_part_an_example_lab_attaches_is_exercised(tmp_path):
    build_tree(tmp_path, parts=["tmp102"], labs=[("tmp102-lab", ["tmp102"])], ledger=ledger(lab=1))
    code, out = run_gate(tmp_path)
    assert code == 0, out
    assert "1 lab-exercised" in out


def test_a_lab_using_an_alias_resolves_to_the_descriptor(tmp_path):
    """`neopixel` is ws2812's canonical spelling; the file is ws2812.yaml."""
    build_tree(
        tmp_path,
        parts=[("ws2812", ["neopixel", "ws2812"])],
        labs=[("strip-lab", ["neopixel"])],
        ledger=ledger(lab=1),
    )
    code, out = run_gate(tmp_path)
    assert code == 0, out


def test_a_lab_scripts_external_system_counts(tmp_path):
    """A lab whose script points at configs/systems/ still ships that board."""
    build_tree(tmp_path, parts=["fxos8700"], ledger=ledger(lab=1))
    (tmp_path / "configs/systems").mkdir()
    (tmp_path / "configs/systems/kw41z.yaml").write_text(
        'chip: "kw41z"\nexternal_devices:\n  - { id: a, type: "fxos8700", connection: i2c0 }\n'
    )
    (tmp_path / "examples/cow").mkdir()
    (tmp_path / "examples/cow/smoke.yaml").write_text(
        'inputs:\n  firmware: "fw.elf"\n  system: "../../configs/systems/kw41z.yaml"\n'
    )
    code, out = run_gate(tmp_path)
    assert code == 0, out


def test_an_integration_test_naming_the_type_is_exercised(tmp_path):
    """Including the escaped form a Rust string holding YAML uses."""
    build_tree(
        tmp_path,
        parts=["uln2803"],
        tests=[("crates/core/tests/bench.rs", r'let y = "    type: \"uln2803\"\n";')],
        ledger=ledger(),
    )
    code, out = run_gate(tmp_path)
    assert code == 0, out
    assert "1 test-exercised" in out


def test_a_longer_spelling_does_not_cover_its_prefix(tmp_path):
    """`ssd1306_128x32` in a test is not a test of `ssd1306`."""
    build_tree(
        tmp_path,
        parts=["ssd1306", "ssd1306_128x32"],
        tests=[("crates/core/tests/oled.rs", 'attach("ssd1306_128x32");')],
        ledger=ledger(),
    )
    code, out = run_gate(tmp_path)
    assert code == 1, out
    assert "part 'ssd1306'" in out


def test_an_in_crate_unit_test_is_not_coverage(tmp_path):
    """`src/` tests mostly parse or construct; they do not attach the part."""
    build_tree(
        tmp_path,
        parts=["74hc32"],
        tests=[("crates/core/src/logic.rs", 'let t = ["74hc32"];')],
        ledger=ledger(),
    )
    code, out = run_gate(tmp_path)
    assert code == 1, out
    assert "part '74hc32'" in out


# ── the ledger ────────────────────────────────────────────────────────────


def test_a_device_only_entry_with_a_reason_passes(tmp_path):
    build_tree(
        tmp_path,
        parts=["xl4015"],
        ledger=ledger(device_only=1, parts="  xl4015:\n    kind: device-only\n    reason: no plant\n"),
    )
    code, out = run_gate(tmp_path)
    assert code == 0, out
    assert "1 device-only" in out


def test_a_device_only_entry_without_a_reason_fails(tmp_path):
    build_tree(
        tmp_path,
        parts=["xl4015"],
        ledger=ledger(device_only=1, parts="  xl4015:\n    kind: device-only\n"),
    )
    code, out = run_gate(tmp_path)
    assert code == 1, out
    assert "needs a concrete `reason`" in out


def test_a_tested_entry_must_name_a_test_that_mentions_the_part(tmp_path):
    entry = "  apa102:\n    kind: tested\n    tests: [crates/core/tests/strip.rs]\n    reason: ctor\n"
    build_tree(
        tmp_path,
        parts=["apa102"],
        tests=[("crates/core/tests/strip.rs", "fn unrelated() {}")],
        ledger=ledger(parts=entry),
    )
    code, out = run_gate(tmp_path)
    assert code == 1, out
    assert "never mentions 'apa102'" in out


def test_a_tested_entry_naming_a_real_test_passes(tmp_path):
    entry = "  apa102:\n    kind: tested\n    tests: [crates/core/tests/strip.rs]\n    reason: ctor\n"
    build_tree(
        tmp_path,
        parts=["apa102"],
        tests=[("crates/core/tests/strip.rs", "let s = apa102(\"PA4\", 2);")],
        ledger=ledger(parts=entry),
    )
    code, out = run_gate(tmp_path)
    assert code == 0, out


def test_an_entry_for_a_derived_part_is_stale(tmp_path):
    entry = "  tmp102:\n    kind: device-only\n    reason: old\n"
    build_tree(
        tmp_path,
        parts=["tmp102"],
        labs=[("tmp102-lab", ["tmp102"])],
        ledger=ledger(device_only=0, lab=1, parts=entry),
    )
    code, out = run_gate(tmp_path)
    assert code == 1, out
    assert "remove it from" in out


def test_an_entry_for_a_deleted_part_is_stale(tmp_path):
    entry = "  gone:\n    kind: device-only\n    reason: old\n"
    build_tree(tmp_path, parts=["tmp102"], labs=[("l", ["tmp102"])], ledger=ledger(lab=1, parts=entry))
    code, out = run_gate(tmp_path)
    assert code == 1, out
    assert "not in configs/devices/" in out


# ── the ratchets ──────────────────────────────────────────────────────────


def test_the_device_only_ceiling_must_be_lowered(tmp_path):
    build_tree(tmp_path, parts=["tmp102"], labs=[("l", ["tmp102"])], ledger=ledger(device_only=3, lab=1))
    code, out = run_gate(tmp_path)
    assert code == 1, out
    assert "Lower `max_device_only:` to 0" in out


def test_a_lab_that_stops_attaching_a_part_trips_the_floor(tmp_path):
    """Lab -> test-only is a Playground regression even though the part is still tested."""
    build_tree(
        tmp_path,
        parts=["tmp102"],
        tests=[("crates/core/tests/t.rs", 'attach("tmp102");')],
        ledger=ledger(lab=1),
    )
    code, out = run_gate(tmp_path)
    assert code == 1, out
    assert "floor is 1" in out


def test_a_new_lab_must_raise_the_floor(tmp_path):
    build_tree(tmp_path, parts=["tmp102"], labs=[("l", ["tmp102"])], ledger=ledger(lab=0))
    code, out = run_gate(tmp_path)
    assert code == 1, out
    assert "Raise `min_lab_exercised:` to 1" in out


def test_a_descriptor_with_no_embedded_row_is_reported(tmp_path):
    build_tree(tmp_path, parts=["tmp102"], labs=[("l", ["tmp102"])], ledger=ledger(lab=1))
    (tmp_path / "configs/devices/stray.yaml").write_text("type: stray\n")
    code, out = run_gate(tmp_path)
    assert code == 1, out
    assert "no EMBEDDED_DEVICES row" in out


# ── the committed tree ────────────────────────────────────────────────────


def test_the_committed_tree_passes_its_own_gate():
    proc = subprocess.run(
        [sys.executable, str(GATE)], capture_output=True, text=True, check=False
    )
    assert proc.returncode == 0, proc.stdout + proc.stderr
