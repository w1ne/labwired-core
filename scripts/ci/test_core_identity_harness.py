"""Exercise actual identity workflow shell with a mock CLI, including missing cells.

No simulator or Cargo runs. Read the literal named shell blocks without adding
PyYAML as a dependency; execute them unchanged against bounded temp artifacts.
"""

import json
import os
import pathlib
import shutil
import subprocess
import tempfile
import textwrap


def literal_run(workflow, name):
    lines = workflow.splitlines(keepends=True)
    start = next(i for i, line in enumerate(lines) if line.strip() == "- name: " + name)
    run = next(i for i in range(start + 1, len(lines)) if lines[i].strip() == "run: |")
    first = run + 1
    indent = len(lines[first]) - len(lines[first].lstrip())
    end = next(
        (
            i
            for i in range(first + 1, len(lines))
            if lines[i].strip() and len(lines[i]) - len(lines[i].lstrip()) < indent
        ),
        len(lines),
    )
    return textwrap.dedent("".join(lines[first:end]))


def test_core_identity_harness():
    ROOT = pathlib.Path(__file__).resolve().parents[2]
    workflow = (ROOT / ".github/workflows/core-identity.yml").read_text()
    collect = literal_run(workflow, "Run the cells")
    compare = literal_run(workflow, "Compare")
    for name, script in [("collect", collect), ("compare", compare)]:
        r = subprocess.run(["bash", "-n"], input=script, text=True, capture_output=True)
        assert r.returncode == 0, (name, r.stderr)
    with tempfile.TemporaryDirectory(prefix="predecode-identity-check-") as tmp:
        t = pathlib.Path(tmp)
        executable = t / "target/release/labwired"
        executable.parent.mkdir(parents=True)
        executable.write_text(textwrap.dedent("""\
    #!/usr/bin/env python3
    import json,os,pathlib,sys
    args=sys.argv[1:]
    chip=args[args.index('--chip')+1]
    assert os.environ['LABWIRED_ARM_SINGLE_STEP'] == ('0' if '--batched' in args else '1')
    data=b'TIER1 UART: simulation reached firmware\\n'
    events=[{'seq':i+1,'cycle':123+i,'bus':'uart0','payload':{'protocol':'uart','direction':'tx','byte':byte}} for i,byte in enumerate(data)]
    pathlib.Path(args[args.index('--bus-trace-out')+1]).write_text(json.dumps(events))
    print('TIER1 UART: simulation reached firmware')
    print('cycle-stamped diagnostic: 123',file=sys.stderr)
    """))
        executable.chmod(0o755)
        for side in ("base", "head"):
            env = dict(
                os.environ,
                STEP_COUNTS="5000000 40000000",
                SIDE=side,
                SIDE_SHA=side + "-exact-sha",
                IDENTITY_SUITE="cortex-m",
            )
            r = subprocess.run(
                ["bash", "-c", collect],
                cwd=t,
                env=env,
                text=True,
                capture_output=True,
                timeout=15,
            )
            assert r.returncode == 0, (side, r.stderr)
            expected = 24 if side == "base" else 12
            exits = list((t / "out").glob("*.exit"))
            assert len(exits) == expected, (side, len(exits))
            assert all("cortex-m-" in p.name for p in exits)
            assert all(p.with_suffix(".bus.json").exists() for p in exits)
            (t / "out").rename(t / f"identity-{side}")

        def check(name, code, word):
            r = subprocess.run(
                ["bash", "-c", compare],
                cwd=t,
                env=dict(
                    os.environ,
                    STEP_COUNTS="5000000 40000000",
                    IDENTITY_SUITE="cortex-m",
                ),
                text=True,
                capture_output=True,
                timeout=15,
            )
            assert r.returncode == code, (
                name,
                r.returncode,
                r.stdout[-1000:],
                r.stderr,
            )
            assert word in r.stdout, (name, word)
            return {"case": name, "exit": r.returncode, "expected_marker": word}

        results = [check("stable identical Cortex-M streams", 0, "judged=48 bad=0")]
        clean = t / "clean"
        clean.mkdir()
        for side in ("base", "head"):
            shutil.copytree(t / f"identity-{side}", clean / f"identity-{side}")

        def reset():
            for side in ("base", "head"):
                shutil.rmtree(t / f"identity-{side}")
                shutil.copytree(clean / f"identity-{side}", t / f"identity-{side}")

        def busfile(side="head", run="p1"):
            return next((t / f"identity-{side}").glob(f"*-{run}.bus.json"))

        bus = busfile()
        events = json.loads(bus.read_text())
        events[0]["cycle"] += 1
        bus.write_text(json.dumps(events))
        results.append(check("changed cycle-stamped bus trace fails", 1, "DIFFERS"))
        reset()
        busfile().unlink()
        results.append(check("missing head bus artifact fails", 1, "FAIL-EXECUTION"))
        reset()
        busfile().write_text("")
        results.append(check("empty bus file fails", 1, "FAIL-EXECUTION"))
        reset()
        busfile().write_text("[]")
        results.append(check("empty JSON event array fails", 1, "FAIL-EXECUTION"))
        reset()
        busfile().write_text("{malformed")
        results.append(check("malformed bus JSON fails", 1, "FAIL-EXECUTION"))
        reset()
        bus = busfile()
        events = json.loads(bus.read_text())
        events[0]["cycle"] = "123"
        bus.write_text(json.dumps(events))
        results.append(check("non-numeric cycle fails", 1, "FAIL-EXECUTION"))
        reset()
        bus = busfile()
        events = json.loads(bus.read_text())
        for e in events:
            e["cycle"] = 0
        bus.write_text(json.dumps(events))
        results.append(check("unstepped zero-cycle UART fails", 1, "FAIL-EXECUTION"))
        reset()
        bus = busfile()
        events = json.loads(bus.read_text())
        events[0]["payload"]["protocol"] = "unknown"
        bus.write_text(json.dumps(events))
        results.append(check("invalid protocol fails", 1, "FAIL-EXECUTION"))
        reset()
        bus = busfile()
        events = json.loads(bus.read_text())
        events[0]["payload"]["direction"] = "sideways"
        bus.write_text(json.dumps(events))
        results.append(check("invalid UART direction fails", 1, "FAIL-EXECUTION"))
        reset()
        next((t / "identity-base").glob("*-p2.exit")).unlink()
        results.append(check("missing second-base exit fails", 1, "FAIL-EXECUTION"))
        reset()
        next((t / "identity-base").glob("*-p2.stdout")).unlink()
        results.append(check("missing second-base stdout fails", 1, "FAIL-EXECUTION"))
        reset()
        busfile("base", "p2").unlink()
        results.append(check("missing second-base bus fails", 1, "FAIL-EXECUTION"))
        reset()
        next((t / "identity-base").glob("*-p2.stderr")).unlink()
        results.append(check("missing second-base stderr fails", 1, "FAIL-MISSING"))
        reset()
        next((t / "identity-base").glob("*-p1.exit")).unlink()
        results.append(check("missing first-base exit fails", 1, "FAIL-INVENTORY"))
        reset()
        missing_cell = next((t / "identity-base").glob("*-p1.exit")).name.removesuffix(
            "-p1.exit"
        )
        for side in ("base", "head"):
            for artifact in (t / f"identity-{side}").glob(missing_cell + "-*"):
                artifact.unlink()
        results.append(check("entire missing Cortex-M cell fails", 1, "FAIL-INVENTORY"))
        reset()
        for p in (t / "identity-base").glob("*-p2.*"):
            if p.suffix == ".exit":
                continue
            if p.name.endswith(".bus.json"):
                events = json.loads(p.read_text())
                for event in events:
                    event["cycle"] += 1
                p.write_text(json.dumps(events))
            else:
                p.write_text("TIER1 different second base control\n")
        results.append(
            check("all observables unstable with only stable exit fails", 1, "bad=1")
        )
        old_workflow = subprocess.check_output(
            [
                "git",
                "show",
                "c35eba55efef5e9aec99c68bbce9d658e18be96b:.github/workflows/core-identity.yml",
            ],
            cwd=ROOT,
            text=True,
        )
        old_compare = literal_run(old_workflow, "Compare")
        old_result = subprocess.run(
            ["bash", "-c", old_compare],
            cwd=t,
            env=dict(
                os.environ, STEP_COUNTS="5000000 40000000", IDENTITY_SUITE="cortex-m"
            ),
            text=True,
            capture_output=True,
            timeout=15,
        )
        assert old_result.returncode == 0 and "judged=12 bad=0" in old_result.stdout
        results.append(
            {
                "case": "original comparator accepts only stable exit despite all observables unstable",
                "exit": old_result.returncode,
                "expected_marker": "judged=12 bad=0",
                "regression_demonstrated": True,
            }
        )
        receipt = {
            "scope": "mock CLI exercises actual workflow shell only; no simulator/firmware identity proven",
            "syntax": "bash -n PASS",
            "cells": {
                "base": 24,
                "head": 12,
                "chips": ["nrf52840", "stm32f405", "stm32l476"],
                "modes": ["step", "batched"],
                "steps": [5000000, 40000000],
            },
            "cases": results,
        }
        print(json.dumps(receipt, indent=2))


def test_selected_suite_inventory():
    root = pathlib.Path(__file__).resolve().parents[2]
    compare = literal_run(
        (root / ".github/workflows/core-identity.yml").read_text(), "Compare"
    )
    cortex = [
        f"cortex-m-{chip}-5000000-{mode}"
        for chip in ("nrf52840", "stm32f405", "stm32l476")
        for mode in ("step", "batched")
    ]
    esp = ["esp32s3-romboot-5000000-step", "esp32s3-romboot-5000000-batched"]
    cases = []
    for suite, cells, judged in (
        ("cortex-m", cortex, 24),
        ("esp32s3", esp, 6),
        ("all", cortex + esp, 30),
    ):
        with tempfile.TemporaryDirectory(prefix="identity-suite-check-") as temporary:
            directory = pathlib.Path(temporary)
            for side, passes in (("base", (1, 2)), ("head", (1,))):
                artifact = directory / f"identity-{side}"
                artifact.mkdir()
                (artifact / "sha").write_text(side + "-exact-sha")
                for cell in cells:
                    for run in passes:
                        prefix = artifact / f"{cell}-p{run}"
                        pathlib.Path(f"{prefix}.exit").write_text("0\n")
                        pathlib.Path(f"{prefix}.stdout").write_text("TIER1 done\n")
                        pathlib.Path(f"{prefix}.stderr").write_text(
                            "retained diagnostic\n"
                        )
                        if cell.startswith("cortex-m-"):
                            events = [
                                {
                                    "seq": i + 1,
                                    "cycle": i + 1,
                                    "bus": "uart0",
                                    "payload": {
                                        "protocol": "uart",
                                        "direction": "tx",
                                        "byte": byte,
                                    },
                                }
                                for i, byte in enumerate(b"TIER1 done\n")
                            ]
                            pathlib.Path(f"{prefix}.bus.json").write_text(
                                json.dumps(events)
                            )

            def execute(steps="5000000", selection=suite):
                return subprocess.run(
                    ["bash", "-c", compare],
                    cwd=directory,
                    env=dict(os.environ, STEP_COUNTS=steps, IDENTITY_SUITE=selection),
                    text=True,
                    capture_output=True,
                    timeout=15,
                )

            result = execute()
            assert result.returncode == 0, result.stdout
            assert f"judged={judged} bad=0" in result.stdout
            cases.append(f"{suite}: complete exact inventory PASS")
            extra = directory / "identity-head" / "unexpected-cell-p1.exit"
            extra.write_text("0\n")
            result = execute()
            assert result.returncode == 1 and "FAIL-INVENTORY" in result.stdout
            cases.append(f"{suite}: unexpected cell rejected")
            extra.unlink()
            for invalid in ("", "0", "-1", "5000000 5000000", "not-a-number"):
                result = execute(steps=invalid)
                assert result.returncode == 1 and "FAIL-INVENTORY" in result.stdout
            result = execute(selection="unknown")
            assert result.returncode == 1 and "FAIL-INVENTORY" in result.stdout
    print(
        json.dumps(
            {
                "suite_inventory_controls": cases,
                "invalid_input_controls": "empty/zero/negative/duplicate/noninteger steps and unknown suite rejected",
            }
        )
    )


if __name__ == "__main__":
    test_core_identity_harness()
    test_selected_suite_inventory()
