#!/usr/bin/env python3
import sys
import subprocess
import json


def get_git_log(from_tag, to_tag):
    """Get git log messages between two tags."""
    cmd = ["git", "log", f"{from_tag}..{to_tag}", "--pretty=format:%s"]
    try:
        result = subprocess.run(cmd, capture_output=True, text=True, check=True)
        return result.stdout.split("\n")
    except subprocess.CalledProcessError:
        return []


def categorize_commits(log_lines):
    """Categorize commits based on conventional commits."""
    categories = {
        "Features": [],
        "Bug Fixes": [],
        "Documentation": [],
        "Refactoring": [],
        "Testing": [],
        "Maintenance": [],
    }

    for line in log_lines:
        line = line.strip()
        if not line:
            continue

        if line.startswith("feat"):
            categories["Features"].append(line)
        elif line.startswith("fix"):
            categories["Bug Fixes"].append(line)
        elif line.startswith("docs"):
            categories["Documentation"].append(line)
        elif line.startswith("refactor") or line.startswith("style"):
            categories["Refactoring"].append(line)
        elif line.startswith("test"):
            categories["Testing"].append(line)
        else:
            categories["Maintenance"].append(line)

    return categories


def generate_markdown(version, date, categories, test_stats, coverage_stats):
    lines = []
    lines.append(f"# Release {version}")
    lines.append(f"**Date:** {date}")
    lines.append("")

    lines.append("## 📊 Quality Report")
    lines.append("| Metric | Status |")
    lines.append("| :--- | :--- |")
    lines.append(
        f"| **Tests** | ✅ {test_stats['passed']} Passed / {test_stats['total']} Total |"
    )
    lines.append(f"| **Coverage** | 📈 {coverage_stats} |")
    lines.append("")

    lines.append("## 🚀 New Features")
    if categories["Features"]:
        for item in categories["Features"]:
            lines.append(f"- {item}")
    else:
        lines.append("- _No major features in this release_")
    lines.append("")

    lines.append("## 🐛 Bug Fixes")
    if categories["Bug Fixes"]:
        for item in categories["Bug Fixes"]:
            lines.append(f"- {item}")
    else:
        lines.append("- _No bug fixes in this release_")
    lines.append("")

    lines.append("<details>")
    lines.append("<summary>Other Changes (Docs, Refactor, Maint)</summary>")
    lines.append("")
    for cat in ["Documentation", "Refactoring", "Testing", "Maintenance"]:
        if categories[cat]:
            lines.append(f"### {cat}")
            for item in categories[cat]:
                lines.append(f"- {item}")
            lines.append("")
    lines.append("</details>")

    return "\n".join(lines)


if __name__ == "__main__":
    if len(sys.argv) < 5:
        print(
            "Usage: generate_release_notes.py <version> <prev_tag> "
            "<test_summary_file> <coverage_summary_file>"
        )
        sys.exit(1)

    version = sys.argv[1]
    prev_tag = sys.argv[2]
    test_json = sys.argv[3]
    cov_file = sys.argv[4]

    log = get_git_log(prev_tag, "HEAD")
    cats = categorize_commits(log)

    t_passed = 0
    t_total = 0
    try:
        with open(test_json, "r") as f:
            d = json.load(f)
            t_passed = d.get("passed", 0)
            t_total = d.get("total", 0)
    except Exception:
        pass

    cov_str = "N/A"
    try:
        with open(cov_file, "r") as f:
            cov_str = f.read().strip()
    except Exception:
        pass

    import datetime

    date_str = datetime.date.today().strftime("%Y-%m-%d")

    md = generate_markdown(
        version, date_str, cats, {"passed": t_passed, "total": t_total}, cov_str
    )
    print(md)
