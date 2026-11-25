"""Every actions/upload-artifact step must bound its retention.

Without an explicit retention-days, GitHub keeps artifacts for 90 days and
they count against the account-wide artifact storage quota.
"""

from pathlib import Path

import pytest
import yaml

REPO_ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = sorted((REPO_ROOT / ".github" / "workflows").glob("*.yml"))
ACTIONS = sorted((REPO_ROOT / ".github" / "actions").glob("*/action.yml"))
MAX_RETENTION_DAYS = 3
# Frozen one-off historical evidence workflows whose external upstream URLs/refs
# no longer resolve if re-triggered on PR; they already cap retention at 7 days
# and do not run on normal PRs.
FROZEN_EVIDENCE_WORKFLOWS = frozenset({
    "pybadge-gcc-source-notices.yml",
    "pybadge-toolchain-notice-evidence.yml",
    "pybadge-toolchain-source-evidence.yml",
    "samd-transfer-state-guest.yml",
})
FROZEN_MAX_RETENTION_DAYS = 7


def _upload_steps():
    for path in WORKFLOWS:
        doc = yaml.safe_load(path.read_text())
        limit = (
            FROZEN_MAX_RETENTION_DAYS
            if path.name in FROZEN_EVIDENCE_WORKFLOWS
            else MAX_RETENTION_DAYS
        )
        for job_id, job in (doc.get("jobs") or {}).items():
            for index, step in enumerate(job.get("steps") or []):
                if str(step.get("uses", "")).startswith("actions/upload-artifact@"):
                    yield pytest.param(step, limit, id=f"{path.name}:{job_id}:{index}")
    for path in ACTIONS:
        doc = yaml.safe_load(path.read_text())
        for index, step in enumerate((doc.get("runs") or {}).get("steps") or []):
            if str(step.get("uses", "")).startswith("actions/upload-artifact@"):
                yield pytest.param(
                    step, MAX_RETENTION_DAYS, id=f"{path.parent.name}/action.yml:{index}"
                )


def _docker_build_steps():
    for path in WORKFLOWS:
        doc = yaml.safe_load(path.read_text())
        wf_env = doc.get("env") or {}
        for job_id, job in (doc.get("jobs") or {}).items():
            job_env = job.get("env") or {}
            for index, step in enumerate(job.get("steps") or []):
                if str(step.get("uses", "")).startswith("docker/build-push-action@"):
                    merged_env = {**wf_env, **job_env, **(step.get("env") or {})}
                    yield pytest.param(merged_env, id=f"{path.name}:{job_id}:{index}")


UPLOAD_STEPS = list(_upload_steps())
DOCKER_BUILD_STEPS = list(_docker_build_steps())


def test_workflows_contain_upload_steps():
    assert UPLOAD_STEPS, "no actions/upload-artifact steps found; parser broken?"


@pytest.mark.parametrize("step,max_days", UPLOAD_STEPS)
def test_upload_artifact_sets_bounded_retention(step, max_days):
    days = (step.get("with") or {}).get("retention-days")
    assert days is not None, "upload-artifact step omits retention-days"
    assert isinstance(days, int) and not isinstance(days, bool), (
        f"retention-days must be an integer literal, got {days!r}"
    )
    assert 1 <= days <= max_days, (
        f"retention-days {days} outside 1..{max_days}"
    )


@pytest.mark.parametrize("env", DOCKER_BUILD_STEPS)
def test_docker_build_push_disables_record_artifact_upload(env):
    val = str(env.get("DOCKER_BUILD_RECORD_UPLOAD", "")).strip().lower()
    assert val == "false", (
        "docker/build-push-action must set DOCKER_BUILD_RECORD_UPLOAD: 'false' "
        "so .dockerbuild records are not uploaded with 90-day default retention"
    )
