#!/usr/bin/env python3
"""Enforce the SR-004 hard/soft CI gate split without a YAML dependency."""

from __future__ import annotations

import pathlib
import re
import sys


WORKFLOW = pathlib.Path(".github/workflows/ci.yml")

HARD_JOBS = {
    "baseline",
    "knowledge-evidence-gate",
    "tenant-isolation-security",
    "frontend-contract",
    "real-llm-smoke-t4",
    "real-llm-redline",
    "skip-gate",
}

SOFT_JOBS = {
    "integration",
    "real-llm",
    "real-llm-recall",
    "real-llm-ops",
    "real-llm-quality",
    "real-llm-adversarial",
}

NIGHTLY_SOFT_JOBS = SOFT_JOBS - {"integration"}


def supports_full_model_run(condition: str) -> bool:
    return (
        "github.event_name == 'schedule'" in condition
        and "github.event.inputs.dispatch_target == 'nightly_full'" in condition
    )


def job_blocks(text: str) -> dict[str, str]:
    lines = text.splitlines()
    try:
        jobs_line = next(i for i, line in enumerate(lines) if line == "jobs:")
    except StopIteration as exc:
        raise ValueError("workflow has no top-level jobs mapping") from exc

    starts: list[tuple[str, int]] = []
    for i in range(jobs_line + 1, len(lines)):
        match = re.fullmatch(r"  ([A-Za-z0-9_-]+):\s*", lines[i])
        if match:
            starts.append((match.group(1), i))

    blocks: dict[str, str] = {}
    for index, (name, start) in enumerate(starts):
        end = starts[index + 1][1] if index + 1 < len(starts) else len(lines)
        blocks[name] = "\n".join(lines[start:end])
    return blocks


def property_value(block: str, key: str) -> str | None:
    match = re.search(rf"(?m)^    {re.escape(key)}:\s*(.*?)\s*$", block)
    return match.group(1) if match else None


def main() -> int:
    text = WORKFLOW.read_text(encoding="utf-8")
    try:
        jobs = job_blocks(text)
    except ValueError as exc:
        print(f"[ci-gate-policy] FAIL: {exc}")
        return 1

    failures: list[str] = []
    required = HARD_JOBS | SOFT_JOBS
    missing = sorted(required - jobs.keys())
    if missing:
        failures.append(f"missing required jobs: {', '.join(missing)}")

    for name in sorted(HARD_JOBS & jobs.keys()):
        value = property_value(jobs[name], "continue-on-error")
        if value not in (None, "false"):
            failures.append(
                f"hard job {name!r} must not enable continue-on-error; found {value!r}"
            )

    for name in sorted(SOFT_JOBS & jobs.keys()):
        if property_value(jobs[name], "continue-on-error") != "true":
            failures.append(f"diagnostic job {name!r} must remain continue-on-error: true")

    for name in sorted(NIGHTLY_SOFT_JOBS & jobs.keys()):
        condition = property_value(jobs[name], "if") or ""
        if not supports_full_model_run(condition):
            failures.append(
                f"variable real-model job {name!r} must support schedule and nightly_full"
            )

    for name in ("real-llm-redline", "skip-gate"):
        if name in jobs:
            condition = property_value(jobs[name], "if") or ""
            if not supports_full_model_run(condition):
                failures.append(
                    f"hard model job {name!r} must support schedule and nightly_full"
                )

    if "real-llm-redline" in jobs:
        needs = property_value(jobs["real-llm-redline"], "needs") or ""
        if "real-llm-adversarial" not in needs:
            failures.append("real-llm-redline must depend on real-llm-adversarial")

    if "skip-gate" in jobs:
        needs = property_value(jobs["skip-gate"], "needs") or ""
        if "real-llm-redline" not in needs:
            failures.append("skip-gate must depend on real-llm-redline")
        if "python3 scripts/check-capability-outcomes.py" not in jobs["skip-gate"]:
            failures.append("skip-gate must execute the typed capability outcome checker")

    if "baseline" in jobs and "python3 scripts/check-ci-gate-policy.py" not in jobs["baseline"]:
        failures.append("baseline must execute this CI gate policy checker")

    print(
        f"[ci-gate-policy] hard={len(HARD_JOBS)} soft={len(SOFT_JOBS)} "
        f"failures={len(failures)}"
    )
    for failure in failures:
        print(f"[ci-gate-policy] FAIL: {failure}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
