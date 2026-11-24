"""Every actions/upload-artifact step must bound its retention.

Without an explicit retention-days, GitHub keeps artifacts for 90 days and
they count against the account-wide artifact storage quota.
"""

from pathlib import Path

import pytest
import yaml

REPO_ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = sorted((REPO_ROOT / ".github" / "workflows").glob("*.yml"))
MAX_RETENTION_DAYS = 30


def _upload_steps():
    for path in WORKFLOWS:
        doc = yaml.safe_load(path.read_text())
        for job_id, job in (doc.get("jobs") or {}).items():
            for index, step in enumerate(job.get("steps") or []):
                if str(step.get("uses", "")).startswith("actions/upload-artifact@"):
                    yield pytest.param(step, id=f"{path.name}:{job_id}:{index}")


UPLOAD_STEPS = list(_upload_steps())


def test_workflows_contain_upload_steps():
    assert UPLOAD_STEPS, "no actions/upload-artifact steps found; parser broken?"


@pytest.mark.parametrize("step", UPLOAD_STEPS)
def test_upload_artifact_sets_bounded_retention(step):
    days = (step.get("with") or {}).get("retention-days")
    assert days is not None, "upload-artifact step omits retention-days"
    assert isinstance(days, int) and not isinstance(days, bool), (
        f"retention-days must be an integer literal, got {days!r}"
    )
    assert 1 <= days <= MAX_RETENTION_DAYS, (
        f"retention-days {days} outside 1..{MAX_RETENTION_DAYS}"
    )
