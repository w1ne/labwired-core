#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Fail closed when selected performance work has not actually succeeded."""
import json
import os
import sys


def boolean(value, name):
    if value not in ("true", "false"):
        raise ValueError(f"{name} must be exactly true or false, got {value!r}")
    return value == "true"


def check(scope_result, required, measurement_result, draft):
    if scope_result != "success":
        raise ValueError(f"performance source admission did not succeed ({scope_result!r})")
    selected = boolean(required, "required")
    is_draft = boolean(draft, "draft")
    if is_draft or not selected:
        if measurement_result != "skipped":
            raise ValueError(f"unselected measurement must be skipped, got {measurement_result!r}")
        return "draft-skipped" if is_draft else "not-required"
    if measurement_result != "success":
        raise ValueError(f"required performance measurement did not succeed ({measurement_result!r})")
    return "measured"


def main():
    inputs = {name: os.environ.get(name, "") for name in
              ("SCOPE_RESULT", "REQUIRED", "MEASUREMENT_RESULT", "DRAFT")}
    try:
        outcome = check(inputs["SCOPE_RESULT"], inputs["REQUIRED"],
                        inputs["MEASUREMENT_RESULT"], inputs["DRAFT"])
    except ValueError as error:
        print(f"::error::{error}", file=sys.stderr)
        return 1
    print(json.dumps(dict(outcome=outcome, **inputs), sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
