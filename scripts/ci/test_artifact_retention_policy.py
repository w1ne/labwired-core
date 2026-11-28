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
MAX_RETENTION_DAYS = 2
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


def test_deploy_docs_prunes_superseded_github_pages_artifacts():
    workflow = yaml.safe_load(
        (REPO_ROOT / ".github" / "workflows" / "deploy-docs.yml").read_text()
    )
    assert (workflow.get("permissions") or {}).get("actions") == "write"

    steps = workflow["jobs"]["deploy"]["steps"]
    prune_index, prune_step = next(
        (index, step)
        for index, step in enumerate(steps)
        if step.get("name") == "Prune superseded github-pages artifacts"
    )
    deploy_index = next(
        index for index, step in enumerate(steps) if step.get("name") == "Build and Deploy"
    )
    assert prune_index < deploy_index
    assert prune_step.get("continue-on-error") is True
    assert str(prune_step.get("uses", "")).startswith("actions/github-script@")

    script = (prune_step.get("with") or {}).get("script", "")
    assert "listArtifactsForRepo" in script
    assert "deleteArtifact" in script
    assert "name: 'github-pages'" in script
    assert "const KEEP = 0;" in script


@pytest.mark.parametrize(
    "workflow_name,required_needs",
    [
        ("core-python-release.yml", {"wheels", "sdist", "linux-arm64-smoke", "publish"}),
        ("core-release.yml", {"build", "linux-smoke", "release", "publish", "release-smoke"}),
    ],
)
def test_release_workflows_clean_up_transient_artifacts(workflow_name, required_needs):
    workflow = yaml.safe_load((REPO_ROOT / ".github" / "workflows" / workflow_name).read_text())
    jobs = workflow.get("jobs") or {}

    for job_id, job in jobs.items():
        for step in job.get("steps") or []:
            if str(step.get("uses", "")).startswith("actions/upload-artifact@"):
                days = (step.get("with") or {}).get("retention-days")
                assert days == 1, f"{workflow_name}:{job_id} upload must set retention-days: 1"
                step_if = str(step.get("if") or "")
                assert "push" in step_if and "tag" in step_if, (
                    f"{workflow_name}:{job_id} upload must gate release-only artifacts on tags"
                )

    cleanup = jobs.get("cleanup-artifacts")
    assert cleanup is not None, f"{workflow_name} missing cleanup-artifacts job"
    assert set(cleanup.get("needs") or []) >= required_needs
    assert str(cleanup.get("if") or "").strip() == "always()"
    assert (cleanup.get("permissions") or {}).get("actions") == "write"
    steps = cleanup.get("steps") or []
    assert any(
        str(step.get("uses", "")).startswith("actions/github-script@")
        and step.get("continue-on-error") is True
        and "listWorkflowRunArtifacts" in (step.get("with") or {}).get("script", "")
        and "deleteArtifact" in (step.get("with") or {}).get("script", "")
        for step in steps
    ), f"{workflow_name}:cleanup-artifacts must delete run artifacts via github-script"


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
