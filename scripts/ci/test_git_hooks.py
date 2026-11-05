"""Exercise hook installation through real Git checkouts and linked worktrees."""

from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]


def run(args, cwd, **kwargs):
    return subprocess.run(args, cwd=cwd, text=True, capture_output=True, check=True, **kwargs)


def make_repo(tmp_path):
    root = tmp_path / "main checkout"
    root.mkdir()
    run(["git", "init", "--quiet"], root)
    for key, value in [("user.name", "Hook Test"), ("user.email", "hook@example.test"),
                       ("commit.gpgsign", "false")]:
        run(["git", "config", key, value], root)
    scripts = root / "scripts"
    scripts.mkdir()
    shutil.copy(REPO_ROOT / "scripts/install-hooks.sh", scripts / "install-hooks.sh")
    return root


def write_hook(root, hook, label):
    path = root / "scripts" / f"{hook}.sh"
    path.write_text(
        f"#!{sys.executable}\nimport json, os, sys\n"
        "with open(os.environ['HOOK_LOG'], 'a') as log:\n"
        f"    log.write(json.dumps([{label!r}, os.getcwd(), sys.argv[1:], sys.stdin.read()]) + '\\n')\n"
        "sys.exit(int(os.environ.get('HOOK_EXIT', '0')))\n"
    )
    path.chmod(0o755)


def test_install_from_linked_worktree_dispatches_each_checkout_and_preserves_io(tmp_path):
    root = make_repo(tmp_path)
    for hook in ["pre-commit", "pre-push"]:
        write_hook(root, hook, "main")
        (root / ".git/hooks" / hook).symlink_to(f"../../scripts/{hook}.sh")
    originals = {name: (root / "scripts" / name).read_bytes()
                 for name in ["pre-commit.sh", "pre-push.sh"]}
    log = tmp_path / "hook.jsonl"
    env = dict(os.environ, HOOK_LOG=str(log))
    run(["bash", "scripts/install-hooks.sh"], root, env=env)
    run(["git", "add", "scripts"], root)
    # A normal commit must enter the installed dispatcher, not just a direct
    # script invocation. stdin is explicitly empty for the fixture hook.
    run(["git", "commit", "--quiet", "-m", "fixture"], root, env=env, input="")
    linked = tmp_path / "linked checkout"
    run(["git", "worktree", "add", "--quiet", "-b", "linked", str(linked)], root)
    for hook in ["pre-commit", "pre-push"]:
        write_hook(linked, hook, "linked")
    run(["bash", "scripts/install-hooks.sh"], linked, env=env)
    run(["git", "add", "scripts"], linked)
    run(["git", "commit", "--quiet", "-m", "linked fixture"], linked, env=env, input="")
    records = [json.loads(line) for line in log.read_text().splitlines()]
    assert [(row[0], row[1]) for row in records] == [
        ("main", str(root)), ("linked", str(linked))
    ]
    # The shared pre-push hook must forward opaque remote args, all stdin and
    # the canonical script's rejection status without consuming any of them.
    push = subprocess.run([str(root / ".git/hooks/pre-push"), "origin", "remote with spaces"],
                          cwd=linked, env=dict(env, HOOK_EXIT="7"), input="refs head\n",
                          text=True, capture_output=True)
    assert push.returncode == 7
    assert json.loads(log.read_text().splitlines()[-1]) == [
        "linked", str(linked), ["origin", "remote with spaces"], "refs head\n"
    ]
    for name, contents in originals.items():
        assert (root / "scripts" / name).read_bytes() == contents
    assert not (root / ".git/hooks/pre-commit").is_symlink()
    assert not (root / ".git/hooks/pre-push").is_symlink()


@pytest.mark.parametrize("custom_symlink", [False, True])
def test_installer_preserves_custom_hooks_without_partial_replacement(tmp_path, custom_symlink):
    root = make_repo(tmp_path)
    for hook in ["pre-commit", "pre-push"]:
        write_hook(root, hook, "main")
    commit = root / ".git/hooks/pre-commit"
    commit.symlink_to("../../scripts/pre-commit.sh")
    custom = root / ".git/hooks/pre-push"
    contents = "#!/bin/sh\nexit 19\n"
    if custom_symlink:
        target = root / "custom-hook"
        target.write_text(contents)
        custom.symlink_to(target)
    else:
        custom.write_text(contents)
    result = subprocess.run(["bash", "scripts/install-hooks.sh"], cwd=root,
                            text=True, capture_output=True)
    assert result.returncode != 0
    assert "Refusing to replace custom hook" in result.stderr
    assert custom.read_text() == contents
    assert custom.is_symlink() == custom_symlink
    assert commit.is_symlink()
