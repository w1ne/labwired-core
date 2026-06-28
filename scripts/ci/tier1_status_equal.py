#!/usr/bin/env python3
"""Exit 0 if two tier1 matrices are equivalent for refresh purposes.

Two matrices are "equal" (exit 0, skip the refresh) iff every cell has the same
status AND the same evidence PRESENCE (whether a run_url exists). The run_url
*value* is ignored, so a re-run that only restamps fresh URLs does not churn
main — but GAINING or LOSING evidence (e.g. a status-only cell that now has a
run_url) IS a change and triggers a refresh. Without the presence check, a
committed matrix with statuses but no run_urls would compare equal to a freshly
stamped one and never get its evidence, leaving the /validation grid all-dots.
"""
import json
import sys


def statuses(path: str) -> dict:
    d = json.loads(open(path).read())
    return {
        chip: {
            cls: (cell.get("status"), bool(cell.get("run_url")))
            for cls, cell in row.items()
        }
        for chip, row in d.items()
    }


def main() -> int:
    try:
        a, b = sys.argv[1], sys.argv[2]
        return 0 if statuses(a) == statuses(b) else 1
    except Exception as e:  # noqa: BLE001
        print(f"tier1_status_equal: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
