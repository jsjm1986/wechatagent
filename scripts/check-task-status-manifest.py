#!/usr/bin/env python3
"""Validate auditable Kiro task states (SR-179).

The historical task markdown is descriptive evidence, not an authority for
delivery status.  This checker requires every task id to appear exactly once in
the JSON manifest and reserves ``verified`` for frozen, production-reachable
work exercised by a blocking CI job.
"""

from __future__ import annotations

import json
import pathlib
import re
import sys
from collections import Counter


ROOT = pathlib.Path(__file__).resolve().parents[1]
MANIFEST = ROOT / ".kiro/specs/task-status-manifest.json"
WORKFLOW = ROOT / ".github/workflows/ci.yml"
ALLOWED_STATES = {
    "planned",
    "implemented",
    "production_wired",
    "verified",
    "partial",
    "sunset_not_shipped",
}
TASK_RE = re.compile(r"^\s*- \[([^]]+)\]\s+(\d+(?:\.\d+)?)\.?\s+", re.MULTILINE)


def workflow_jobs(text: str) -> dict[str, str]:
    lines = text.splitlines()
    jobs_at = next((i for i, line in enumerate(lines) if line == "jobs:"), None)
    if jobs_at is None:
        return {}
    starts: list[tuple[str, int]] = []
    for i in range(jobs_at + 1, len(lines)):
        match = re.fullmatch(r"  ([A-Za-z0-9_-]+):\s*", lines[i])
        if match:
            starts.append((match.group(1), i))
    return {
        name: "\n".join(lines[start : starts[index + 1][1] if index + 1 < len(starts) else len(lines)])
        for index, (name, start) in enumerate(starts)
    }


def split_evidence(value: str) -> tuple[pathlib.Path, str | None]:
    path_text, separator, selector = value.partition("#")
    return ROOT / path_text, selector if separator else None


def validate_evidence(values: object, label: str, failures: list[str]) -> list[str]:
    if not isinstance(values, list) or not all(isinstance(item, str) and item for item in values):
        failures.append(f"{label} must be a list of non-empty strings")
        return []
    for value in values:
        path, selector = split_evidence(value)
        if not path.is_file():
            failures.append(f"{label} references missing file: {value}")
            continue
        if selector and selector not in path.read_text(encoding="utf-8"):
            failures.append(f"{label} selector not found: {value}")
    return values


def main() -> int:
    failures: list[str] = []
    try:
        manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        print(f"[task-status] FAIL: cannot read manifest: {error}")
        return 1

    if manifest.get("schemaVersion") != 1:
        failures.append("schemaVersion must equal 1")
    specs = manifest.get("specs")
    records = manifest.get("records")
    if not isinstance(specs, dict) or not specs:
        failures.append("specs must be a non-empty object")
        specs = {}
    if not isinstance(records, list):
        failures.append("records must be an array")
        records = []

    expected: set[str] = set()
    for spec, relative in specs.items():
        path = ROOT / str(relative)
        if not path.is_file():
            failures.append(f"spec {spec!r} references missing task file: {relative}")
            continue
        text = path.read_text(encoding="utf-8")
        if re.search(r"(?m)^\s*- \[x\]", text, re.IGNORECASE):
            failures.append(f"{relative} still uses authoritative-looking [x] markers")
        ids = TASK_RE.findall(text)
        if not ids:
            failures.append(f"{relative} contains no parseable task ids")
        expected.update(f"{spec}:{task_id}" for _, task_id in ids)

    jobs = workflow_jobs(WORKFLOW.read_text(encoding="utf-8"))
    seen: list[str] = []
    for index, record in enumerate(records):
        label = f"records[{index}]"
        if not isinstance(record, dict):
            failures.append(f"{label} must be an object")
            continue
        spec = record.get("spec")
        state = record.get("state")
        task_ids = record.get("taskIds")
        note = record.get("note")
        if spec not in specs:
            failures.append(f"{label}.spec is unknown: {spec!r}")
        if state not in ALLOWED_STATES:
            failures.append(f"{label}.state is invalid: {state!r}")
        if not isinstance(task_ids, list) or not task_ids or not all(isinstance(item, str) for item in task_ids):
            failures.append(f"{label}.taskIds must be a non-empty string array")
            task_ids = []
        qualified = [f"{spec}:{task_id}" for task_id in task_ids]
        seen.extend(qualified)
        for task in qualified:
            if task not in expected:
                failures.append(f"{label} references unknown task id: {task}")

        implementation = validate_evidence(record.get("implementation", []), f"{label}.implementation", failures)
        production = validate_evidence(record.get("productionEntrypoints", []), f"{label}.productionEntrypoints", failures)
        tests = validate_evidence(record.get("tests", []), f"{label}.tests", failures)
        ci_jobs = record.get("ciJobs", [])
        if not isinstance(ci_jobs, list) or not all(isinstance(item, str) for item in ci_jobs):
            failures.append(f"{label}.ciJobs must be a string array")
            ci_jobs = []

        if state in {"implemented", "production_wired", "verified"} and not implementation:
            failures.append(f"{label} state {state} requires implementation evidence")
        if state in {"production_wired", "verified"} and not production:
            failures.append(f"{label} state {state} requires a production entrypoint")
        if state in {"partial", "sunset_not_shipped"} and not isinstance(note, str):
            failures.append(f"{label} state {state} requires a note")
        if state == "verified":
            frozen = record.get("frozenCommit")
            if not isinstance(frozen, str) or not re.fullmatch(r"[0-9a-f]{40}", frozen):
                failures.append(f"{label} verified state requires a 40-char frozenCommit")
            if not tests or not ci_jobs:
                failures.append(f"{label} verified state requires tests and ciJobs")
            for job in ci_jobs:
                block = jobs.get(job)
                if block is None:
                    failures.append(f"{label} references missing CI job: {job}")
                elif re.search(r"(?m)^    continue-on-error:\s*true\s*$", block):
                    failures.append(f"{label} verified state references soft CI job: {job}")
                elif not any(
                    selector and selector in block or path.stem in block
                    for test in tests
                    for path, selector in [split_evidence(test)]
                ):
                    failures.append(f"{label} CI job {job} does not name a bound test artifact")

    counts = Counter(seen)
    duplicates = sorted(task for task, count in counts.items() if count > 1)
    missing = sorted(expected - counts.keys())
    if duplicates:
        failures.append("duplicate task coverage: " + ", ".join(duplicates))
    if missing:
        failures.append("missing task coverage: " + ", ".join(missing))

    state_counts = Counter(record.get("state") for record in records if isinstance(record, dict))
    print(
        f"[task-status] expected={len(expected)} covered={len(counts)} "
        f"verified={state_counts['verified']} failures={len(failures)}"
    )
    for failure in failures:
        print(f"[task-status] FAIL: {failure}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
