#!/usr/bin/env python3
"""Refresh one auto-merge PR at a time without bypassing checks."""
import datetime as dt
import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request

REPO = os.environ["GITHUB_REPOSITORY"]
TOKEN = os.environ["GITHUB_TOKEN"]
DRY_RUN = os.environ.get("DRY_RUN", "false").lower() == "true"
REQUIRED = {"pr-gate", "release-runner-contract", "browser-layer", "pr-core-feature-off"}
API = "https://api.github.com"
HEADERS = {
    "Accept": "application/vnd.github+json",
    "Authorization": f"Bearer {TOKEN}",
    "X-GitHub-Api-Version": "2022-11-28",
    "User-Agent": "labwired-core-safe-merge-queue",
}

def request(path, method="GET", payload=None):
    data = json.dumps(payload).encode() if payload is not None else None
    req = urllib.request.Request(API + path, data=data, headers=HEADERS, method=method)
    with urllib.request.urlopen(req, timeout=30) as response:
        return json.load(response)

def pull_requests():
    page = 1
    while True:
        batch = request(f"/repos/{REPO}/pulls?state=open&per_page=100&page={page}")
        yield from batch
        if len(batch) < 100:
            break
        page += 1

def head_checks_pass(sha):
    checks = request(f"/repos/{REPO}/commits/{sha}/check-runs?per_page=100")
    runs = checks.get("check_runs", [])
    if not runs or len(runs) != checks.get("total_count", 0):
        return False
    names = {run["name"] for run in runs if run["status"] == "completed" and run["conclusion"] == "success"}
    if not REQUIRED <= names:
        return False
    return all(run["status"] == "completed" and run["conclusion"] in ("success", "skipped") for run in runs)

def main():
    now = dt.datetime.now(dt.timezone.utc)
    pending = []
    for summary in pull_requests():
        if summary.get("draft") or not summary.get("auto_merge"):
            continue
        if summary.get("base", {}).get("ref") != "main":
            continue
        if summary.get("head", {}).get("repo", {}).get("full_name") != REPO:
            continue
        number = summary["number"]
        pr = request(f"/repos/{REPO}/pulls/{number}")
        if not pr.get("mergeable") or pr.get("mergeable_state") == "dirty":
            continue
        if pr.get("changed_files", 1000) > 40 or pr.get("commits", 1000) > 25:
            continue
        updated = dt.datetime.fromisoformat(pr["updated_at"].replace("Z", "+00:00"))
        if now - updated < dt.timedelta(minutes=15):
            continue
        sha = pr["head"]["sha"]
        if not head_checks_pass(sha):
            continue
        base_sha = pr["base"]["sha"]
        compare = request(f"/repos/{REPO}/compare/{base_sha}...{sha}")
        if compare.get("behind_by", 0) < 1:
            continue
        pending.append((pr["changed_files"], pr["created_at"], number, sha))
    if not pending:
        print("No eligible green, auto-merge-enabled, behind-main PR to update.")
        return
    _, _, number, sha = sorted(pending)[0]
    print(f"Selected PR #{number}, expected head {sha}; dry_run={DRY_RUN}")
    if DRY_RUN:
        return
    try:
        result = request(f"/repos/{REPO}/pulls/{number}/update-branch",
                         method="PUT", payload={"expected_head_sha": sha})
        print(f"GitHub accepted branch refresh for PR #{number}: {result.get('message')}")
    except urllib.error.HTTPError as error:
        print(f"Update rejected with HTTP {error.code}; no protection bypass attempted.", file=sys.stderr)
        raise

if __name__ == "__main__":
    main()